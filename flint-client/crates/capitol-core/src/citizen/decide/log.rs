//! The decision log (C3.1, plan D4's *The log*): what makes the cascade
//! safe to grow.
//!
//! Each citizen writes a JSONL file of [`Record`]s, one a line, tagged by
//! `record`:
//!
//! - **`start`**, when a run opens the log (and at the top of each file
//!   after a rotation): the citizen, its role (its birth trade), its
//!   calling (Callings K1.1; none in a log from before them) and its stack
//!   of layers. Decision ids count up from 1 within a run.
//! - **`decision`**: the [`Situation`] with its menu, and the [`Ruling`]:
//!   each layer's verdict on the trail (settled, unsure, escalated, off the
//!   menu, slow, pending), with its choice and how long it took, and the
//!   decision that stood.
//! - **`outcome`**, once known: what came of the decision ([`Fate`]), the
//!   game seconds it took, and the change in Strands and goods since it was
//!   made. A sale or a purchase at the hall is waited out within its
//!   errand, so its fill is in that change.
//! - **`fill`**: a standing bid that filled, or a stall that sold, after
//!   the decision that put it up had its outcome.
//!
//! [`replay`](super::replay) runs any stack of layers over the decisions
//! offline. The log stays local (beside the roster, git-ignored), holds no
//! keys, and rotates: past [`Log::open`]'s limit the file moves to
//! `<name>.1.jsonl`, the one before to `.2`, and so on to `keep`.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Held, Ruling, Situation};
use crate::citizen::calling::Calling;

/// The size a log grows to before it rotates, unless the town says
/// otherwise.
pub const LOG_LIMIT: u64 = 4 << 20;
/// How many rotated files a log keeps.
pub const LOG_KEEP: usize = 2;

/// One line of a decision log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    Start {
        /// Unix ms
        t: i64,
        /// When the run opened the log, unix ms: decision ids are the run's.
        run: i64,
        citizen: String,
        role: String,
        /// What it would rather be doing (K1.1). A log from before
        /// callings has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        calling: Option<Calling>,
        layers: Vec<String>,
    },
    /// Boxed, so the other records aren't the size of a decision (#264).
    Decision {
        t: i64,
        id: u64,
        situation: Box<Situation>,
        ruling: Box<Ruling>,
    },
    Outcome {
        t: i64,
        id: u64,
        #[serde(flatten)]
        fate: Fate,
        /// Game seconds from the decision to its outcome
        took: f64,
        /// Strands gained (lost, if negative) since the decision
        #[serde(default, skip_serializing_if = "is_zero")]
        strands: i64,
        /// Goods gained or lost since the decision, by item
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        items: BTreeMap<String, i32>,
    },
    Fill {
        t: i64,
        id: u64,
        quantity: i64,
        /// Strands that changed hands
        strands: i64,
    },
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// What came of a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Fate {
    /// What it chose was done: the step taken, the order waited out, the
    /// bid posted, the stall stocked, the time passed.
    Done,
    Failed { why: String },
    /// It chose to let it be: keep the goods, or gather rather than buy.
    Passed,
    /// Something went first (a trade before the work, the market's hours),
    /// or a later decision on the same question took its place.
    Superseded,
    /// A P2P trade it answered went through.
    Traded,
    /// It declined the trade.
    Declined,
    /// The trade ended otherwise: they walked off, or nobody offered.
    Ended { reason: String },
}

impl Fate {
    /// For a report: `done`, `failed`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Fate::Done => "done",
            Fate::Failed { .. } => "failed",
            Fate::Passed => "passed",
            Fate::Superseded => "superseded",
            Fate::Traded => "traded",
            Fate::Declined => "declined",
            Fate::Ended { .. } => "ended",
        }
    }
}

/// The change from `before` to `after`: Strands, and the goods that moved.
pub fn change(before: &Held, after: &Held) -> (i64, BTreeMap<String, i32>) {
    let mut items = BTreeMap::new();
    for item in before.items.keys().chain(after.items.keys()) {
        let d = after.count(item) - before.count(item);
        if d != 0 {
            items.insert(item.clone(), d);
        }
    }
    (after.strands - before.strands, items)
}

/// Unix ms now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A citizen's decision log, open for appending.
pub struct Log {
    path: PathBuf,
    file: File,
    written: u64,
    limit: u64,
    keep: usize,
    /// The run's `start`, written again at the top of each new file
    start: Record,
    next: u64,
    /// The first failure to write, said once
    failed: bool,
}

impl Log {
    /// Opens (or makes) the log at `path` for a run of `citizen`, appending
    /// its `start`. It rotates past `limit` bytes, keeping `keep` old files.
    pub fn open(
        path: &Path,
        citizen: &str,
        role: &str,
        calling: Option<&Calling>,
        layers: &[String],
        limit: u64,
        keep: usize,
    ) -> io::Result<Log> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let written = file.metadata()?.len();
        let t = now_ms();
        let start = Record::Start {
            t,
            run: t,
            citizen: citizen.to_string(),
            role: role.to_string(),
            calling: calling.cloned(),
            layers: layers.to_vec(),
        };
        let mut log = Log {
            path: path.to_path_buf(),
            file,
            written,
            limit,
            keep,
            start: start.clone(),
            next: 1,
            failed: false,
        };
        log.write(&start);
        Ok(log)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Logs a decision, and returns its id.
    pub fn decision(&mut self, situation: &Situation, ruling: &Ruling) -> u64 {
        let id = self.next;
        self.next += 1;
        self.write(&Record::Decision {
            t: now_ms(),
            id,
            situation: Box::new(situation.clone()),
            ruling: Box::new(ruling.clone()),
        });
        id
    }

    /// Appends `record` as a line, rotating first if the file is full. A
    /// failure is said once and otherwise ignored: a citizen never stops
    /// for its log.
    pub fn write(&mut self, record: &Record) {
        let mut line = match serde_json::to_string(record) {
            Ok(line) => line,
            Err(e) => return self.fail(&e.to_string()),
        };
        line.push('\n');
        if self.written > 0 && self.written + line.len() as u64 > self.limit && !matches!(record, Record::Start { .. }) {
            if let Err(e) = self.rotate() {
                return self.fail(&format!("rotating: {e}"));
            }
        }
        match self.file.write_all(line.as_bytes()) {
            Ok(()) => self.written += line.len() as u64,
            Err(e) => self.fail(&e.to_string()),
        }
    }

    /// `<name>.jsonl` to `<name>.1.jsonl`, each older one up a number, the
    /// oldest past `keep` gone; then a fresh file, headed by the run's
    /// `start`.
    fn rotate(&mut self) -> io::Result<()> {
        let _ = fs::remove_file(rotated(&self.path, self.keep));
        for n in (1..self.keep).rev() {
            let from = rotated(&self.path, n);
            if from.exists() {
                fs::rename(&from, rotated(&self.path, n + 1))?;
            }
        }
        if self.keep > 0 {
            fs::rename(&self.path, rotated(&self.path, 1))?;
        } else {
            fs::remove_file(&self.path)?;
        }
        self.file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        self.written = 0;
        let start = self.start.clone();
        self.write(&start);
        Ok(())
    }

    fn fail(&mut self, why: &str) {
        if !self.failed {
            self.failed = true;
            eprintln!("{}: the decision log stopped: {why}", self.path.display());
        }
    }
}

/// The `n`th rotated file of `path`: `percy-sawyer.jsonl` to
/// `percy-sawyer.2.jsonl`. 0 is `path` itself.
pub fn rotated(path: &Path, n: usize) -> PathBuf {
    if n == 0 {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{stem}.{n}.jsonl"))
}

/// A citizen's log file in `dir`: its name, lowercased, with dashes
/// (`Percy Sawyer` to `percy-sawyer.jsonl`).
pub fn file_for(dir: &Path, name: &str) -> PathBuf {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    dir.join(format!("{}.jsonl", if slug.is_empty() { "citizen" } else { slug }))
}

/// The records of the log at `path`, in order. A line that won't read (the
/// half-written last line of a run that was killed, say) is an `Err` with
/// its line number, and the rest still read.
pub fn read(path: &Path) -> io::Result<impl Iterator<Item = Result<Record, String>>> {
    let file = File::open(path)?;
    Ok(BufReader::new(file)
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.as_ref().is_ok_and(|l| l.trim().is_empty()))
        .map(|(n, line)| {
            let line = line.map_err(|e| format!("line {}: {e}", n + 1))?;
            serde_json::from_str(&line).map_err(|e| format!("line {}: {e}", n + 1))
        }))
}

/// The logs `paths` name, oldest first: a file as it is, a directory as its
/// `.jsonl` files, each citizen's rotated files before its current one.
pub fn files(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if !path.is_dir() {
            out.push(path.clone());
            continue;
        }
        let mut found: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        // percy-sawyer.2, .1, then percy-sawyer
        found.sort_by_key(|p| {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match stem.rsplit_once('.').and_then(|(name, n)| Some((name.to_string(), n.parse::<usize>().ok()?))) {
                Some((name, n)) => (name, std::cmp::Reverse(n)),
                None => (stem, std::cmp::Reverse(0)),
            }
        });
        out.extend(found);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::{Cascade, Choice, Place, Question, Temperament, When, Who};
    use super::*;
    use std::time::Instant;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capitol-decisions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn situation() -> Situation {
        Situation {
            who: Who {
                name: "Percy Sawyer".into(),
                role: "gatherer-wood".into(),
                temperament: Temperament {
                    thrift: 0.123_456_79,
                    risk: 0.7,
                    sociability: 0.1,
                    patience: 0.9,
                },
                calling: None,
            },
            at: Place {
                x: 6012.25,
                y: 5990.5,
                zone: "rural".into(),
            },
            when: When {
                time_of_day: Some(0.41),
                hours: 2.5,
            },
            holdings: Held::default(),
            market: Vec::new(),
            question: Question::FreeTime,
            menu: vec![Choice::Rest, Choice::Hall],
            carry_on: Some(0),
        }
    }

    #[test]
    fn every_record_goes_through_a_line_and_back() {
        let s = situation();
        let ruling = Cascade::rules().ask(&s, Instant::now());
        let mut items = BTreeMap::new();
        items.insert("log".to_string(), -10);
        let records = [
            Record::Start {
                t: 1,
                run: 1,
                citizen: "Percy Sawyer".into(),
                role: "gatherer-wood".into(),
                calling: Some(Calling::single(crate::sim::Role::Gatherer(crate::sim::Gather::Wood))),
                layers: vec!["rules".into()],
            },
            Record::Decision {
                t: 2,
                id: 1,
                situation: Box::new(s),
                ruling: Box::new(ruling),
            },
            Record::Outcome {
                t: 3,
                id: 1,
                fate: Fate::Failed { why: "no logs".into() },
                took: 61.5,
                strands: 50,
                items,
            },
            Record::Outcome {
                t: 3,
                id: 2,
                fate: Fate::Passed,
                took: 0.0,
                strands: 0,
                items: BTreeMap::new(),
            },
            Record::Fill {
                t: 4,
                id: 1,
                quantity: 3,
                strands: 21,
            },
        ];
        for r in &records {
            let line = serde_json::to_string(r).unwrap();
            assert_eq!(&serde_json::from_str::<Record>(&line).unwrap(), r, "{line}");
        }
        let line = serde_json::to_string(&records[2]).unwrap();
        assert!(line.contains(r#""record":"outcome","#) && line.contains(r#""result":"failed","why":"no logs""#), "{line}");
        // Nothing moved, nothing written
        assert_eq!(serde_json::to_string(&records[3]).unwrap(), r#"{"record":"outcome","t":3,"id":2,"result":"passed","took":0.0}"#);
        // A layer's turn says what it chose and how long it took
        let line = serde_json::to_string(&records[1]).unwrap();
        assert!(line.contains(r#""layer":"rules","outcome":"settled","#) && line.contains(r#""micros":"#), "{line}");
    }

    #[test]
    fn a_log_from_before_callings_still_reads() {
        // A start and a situation's who as C3.1 to K0.3 wrote them
        let start = r#"{"record":"start","t":1,"run":1,"citizen":"Percy Sawyer","role":"gatherer-wood","layers":["rules"]}"#;
        let Record::Start { calling, role, .. } = serde_json::from_str(start).unwrap() else { panic!() };
        assert_eq!((calling, role.as_str()), (None, "gatherer-wood"));
        let mut s = situation();
        s.who.calling = None;
        let line = serde_json::to_string(&s).unwrap();
        assert!(!line.contains("calling"), "{line}");
        assert_eq!(serde_json::from_str::<Situation>(&line).unwrap(), s);
        // Its stint is its role's alone
        assert_eq!(s.who.calling(), Some(Calling::single(crate::sim::Role::Gatherer(crate::sim::Gather::Wood))));
    }

    #[test]
    fn a_temperament_survives_the_log_exactly() {
        // The rules weigh the temperament, so replay needs it to the bit
        let mut x: u32 = 1;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let v = (x >> 8) as f32 / (1u32 << 24) as f32;
            let back: f32 = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
            assert_eq!(back.to_bits(), v.to_bits(), "{v}");
        }
    }

    #[test]
    fn a_log_rotates_and_each_file_starts_with_its_run() {
        let dir = temp("rotate");
        let path = file_for(&dir, "Percy Sawyer");
        assert_eq!(path.file_name().unwrap(), "percy-sawyer.jsonl");
        let mut log = Log::open(&path, "Percy Sawyer", "gatherer-wood", None, &["rules".into()], 2000, 2).unwrap();
        let s = situation();
        let ruling = Cascade::rules().ask(&s, Instant::now());
        let ids: Vec<u64> = (0..20).map(|_| log.decision(&s, &ruling)).collect();
        assert_eq!(ids, (1..=20).collect::<Vec<_>>());
        assert!(rotated(&path, 1).exists() && rotated(&path, 2).exists());
        assert!(!rotated(&path, 3).exists(), "only two are kept");
        for file in files(std::slice::from_ref(&dir)).unwrap() {
            assert!(fs::metadata(&file).unwrap().len() <= 2000 + 1000, "{}", file.display());
            let first = read(&file).unwrap().next().unwrap().unwrap();
            assert!(matches!(first, Record::Start { ref citizen, .. } if citizen == "Percy Sawyer"), "{}", file.display());
        }
        // Oldest first, and the ids run on across them
        let found = files(std::slice::from_ref(&dir)).unwrap();
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["percy-sawyer.2.jsonl", "percy-sawyer.1.jsonl", "percy-sawyer.jsonl"]);
        let ids: Vec<u64> = found
            .iter()
            .flat_map(|f| read(f).unwrap())
            .filter_map(|r| match r.unwrap() {
                Record::Decision { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert!(ids.windows(2).all(|w| w[1] == w[0] + 1), "{ids:?}");
        assert_eq!(ids.last(), Some(&20));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_torn_line_is_an_error_and_the_rest_still_read() {
        let dir = temp("torn");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");
        let fill = Record::Fill {
            t: 1,
            id: 1,
            quantity: 1,
            strands: 2,
        };
        let line = serde_json::to_string(&fill).unwrap();
        fs::write(&path, format!("{line}\n{{\"record\":\"fi\n\n{line}\n")).unwrap();
        let read: Vec<_> = read(&path).unwrap().collect();
        assert_eq!(read.len(), 3);
        assert!(read[1].as_ref().is_err_and(|e| e.starts_with("line 2")));
        assert_eq!(read[2], Ok(fill));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_change_counts_what_moved() {
        let mut before = Held {
            strands: 100,
            ..Held::default()
        };
        before.items.insert("log".into(), 10);
        before.items.insert("fiber".into(), 3);
        let mut after = Held {
            strands: 160,
            ..Held::default()
        };
        after.items.insert("log".into(), 4);
        after.items.insert("fiber".into(), 3);
        after.items.insert("plank".into(), 1);
        let (strands, items) = change(&before, &after);
        assert_eq!(strands, 60);
        assert_eq!(items.into_iter().collect::<Vec<_>>(), [("log".to_string(), -6), ("plank".to_string(), 1)]);
    }
}

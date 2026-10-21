//! Live tunables (the Observatory's D6.1, #375): the economy's rates and fees,
//! changed while the world runs.
//!
//! [`Live`] holds the world's [`Tunables`] in an `ArcSwap`, so whatever charges
//! or grows by them reads the current value without a lock: the exchange, the
//! market's listing fee, a stall's rent, the hourly tax, a deed's price, a
//! node's regrowth and a gather's yield. [`load`] reads the `tunables` table at
//! boot. It holds only the knobs that differ from their defaults, so a default
//! changed in code reaches every world that never moved it.
//!
//! [`update`] is the one way to change them (`PUT /admin/tunables`). It checks
//! every value before setting any, writes the rows and a `tunable_changed`
//! annotation per knob in one transaction, then swaps the new set in. When a
//! fee moved, every socket hears `fees_changed`, so a client's previews follow.

use std::sync::Arc;

use arc_swap::ArcSwap;
use capitol_protocol::annotation::AnnotationKind;
use capitol_protocol::tunables::{Family, Knob, TunableError, Tunables, TunablesUpdate};
use sqlx::PgPool;
use tokio::sync::{broadcast, Mutex};

use super::annotations;
use super::events::GameEvent;

/// The world's tunables. Clones share one value.
#[derive(Clone, Default)]
pub struct Live {
    current: Arc<ArcSwap<Tunables>>,
    /// Held across an update, so two at once can't each overwrite the other's
    writing: Arc<Mutex<()>>,
}

impl Live {
    pub fn new(tunables: Tunables) -> Self {
        Live { current: Arc::new(ArcSwap::from_pointee(tunables)), writing: Arc::default() }
    }

    /// The tunables now. Cheap: read it where it's charged, not once at startup.
    pub fn get(&self) -> Arc<Tunables> {
        self.current.load_full()
    }
}

/// The zones and resources the families' knobs may name: zones with deeds,
/// and the content's resource types
#[derive(Debug, Clone, Default)]
pub struct Known {
    pub zones: Vec<String>,
    pub resources: Vec<String>,
}

impl Known {
    /// Read the zones from the database; the resources are the caller's
    /// (`GameState::resource_types`)
    pub async fn read(db: &PgPool, resources: impl IntoIterator<Item = String>) -> Result<Known, sqlx::Error> {
        let zones: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM zones WHERE has_plots AND deed_cost IS NOT NULL ORDER BY ring_order",
        )
        .fetch_all(db)
        .await?;
        let mut resources: Vec<String> = resources.into_iter().collect();
        resources.sort();
        Ok(Known { zones, resources })
    }

    pub fn has(&self, family: Family, id: &str) -> bool {
        match family {
            Family::Zone => self.zones.iter().any(|z| z == id),
            Family::Resource => self.resources.iter().any(|r| r == id),
        }
    }
}

/// The saved tunables over the defaults. A row naming a knob that's gone, a
/// zone or resource that's gone, or a value now out of range is skipped with a
/// warning: the world starts on the default for it.
pub async fn load(db: &PgPool, known: &Known) -> Result<Tunables, sqlx::Error> {
    let rows: Vec<(String, f64)> = sqlx::query_as("SELECT key, value FROM tunables ORDER BY key").fetch_all(db).await?;
    let mut tunables = Tunables::default();
    for (key, value) in rows {
        match tunables.set(&key, value, |family, id| known.has(family, id)) {
            Ok(set) => tracing::info!("Tunable {key} is {set}"),
            Err(e) => tracing::warn!("Skipping the saved tunable {key} = {value}: {e:?}"),
        }
    }
    Ok(tunables)
}

/// One knob an update moved
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub key: String,
    pub from: f64,
    pub to: f64,
}

#[derive(Debug)]
pub enum UpdateError {
    /// Each refused key, with why. Nothing was changed.
    Refused(Vec<(String, TunableError)>),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for UpdateError {
    fn from(e: sqlx::Error) -> Self {
        UpdateError::Db(e)
    }
}

impl UpdateError {
    /// For a 400: "property_tax_rate: 0.9 is outside 0 to 0.5"
    pub fn message(&self, update: &TunablesUpdate) -> String {
        match self {
            UpdateError::Refused(refused) => refused
                .iter()
                .map(|(key, e)| {
                    let value = update.values.get(key).copied().unwrap_or(f64::NAN);
                    match e {
                        TunableError::UnknownKey => format!("{key}: no such tunable"),
                        TunableError::NotANumber => format!("{key}: not a number"),
                        TunableError::OutOfRange { min, max } => format!("{key}: {value} is outside {min} to {max}"),
                    }
                })
                .collect::<Vec<_>>()
                .join("; "),
            UpdateError::Db(e) => e.to_string(),
        }
    }
}

/// The annotation for a change: "Property tax a week: 2% → 5%"
fn describe(change: &Change) -> String {
    let Some((knob, id)) = Knob::find(&change.key) else {
        return format!("{}: {} → {}", change.key, change.from, change.to);
    };
    let label = match id {
        Some(id) => format!("{}: {id}", knob.label),
        None => knob.label.to_string(),
    };
    format!("{label}: {} → {}", knob.unit.format(change.from), knob.unit.format(change.to))
}

/// Set the knobs in `update`, all of them or none. Writes what changed to
/// `tunables` with an annotation each, swaps the new values in, and tells every
/// socket if a fee moved. Returns what changed: a value set to what it already
/// was isn't a change.
pub async fn update(
    db: &PgPool,
    live: &Live,
    update: &TunablesUpdate,
    known: &Known,
    events: &broadcast::Sender<GameEvent>,
) -> Result<Vec<Change>, UpdateError> {
    let _writing = live.writing.lock().await;
    let before = live.get();
    let mut next = (*before).clone();
    let mut refused = Vec::new();
    for (key, value) in &update.values {
        if let Err(e) = next.set(key, *value, |family, id| known.has(family, id)) {
            refused.push((key.clone(), e));
        }
    }
    if !refused.is_empty() {
        return Err(UpdateError::Refused(refused));
    }

    let changes: Vec<Change> = update
        .values
        .keys()
        .filter_map(|key| {
            let (from, to) = (before.get(key)?, next.get(key)?);
            (from != to).then(|| Change { key: key.clone(), from, to })
        })
        .collect();
    if changes.is_empty() {
        return Ok(changes);
    }

    let mut tx = db.begin().await?;
    for change in &changes {
        let default = Knob::find(&change.key).map(|(knob, _)| knob.default);
        if Some(change.to) == default {
            sqlx::query("DELETE FROM tunables WHERE key = $1").bind(&change.key).execute(&mut *tx).await?;
        } else {
            sqlx::query(
                "INSERT INTO tunables (key, value) VALUES ($1, $2)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
            )
            .bind(&change.key)
            .bind(change.to)
            .execute(&mut *tx)
            .await?;
        }
        annotations::record(&mut *tx, AnnotationKind::TunableChanged, &describe(change)).await?;
    }
    tx.commit().await?;

    let fees = next.fees;
    live.current.store(Arc::new(next));
    for change in &changes {
        tracing::info!("Tunable {} changed: {} → {}", change.key, change.from, change.to);
    }
    if fees != before.fees {
        let _ = events.send(GameEvent::FeesChanged { fees });
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn known() -> Known {
        Known { zones: vec!["urban".into(), "rural".into()], resources: vec!["grass".into(), "clay".into()] }
    }

    fn values(pairs: &[(&str, f64)]) -> TunablesUpdate {
        TunablesUpdate { values: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<_, _>>() }
    }

    #[test]
    fn a_change_reads_in_its_units() {
        let change = Change { key: "property_tax_rate".into(), from: 0.02, to: 0.05 };
        assert_eq!(describe(&change), "Property tax a week: 2% → 5%");
        let change = Change { key: "yield.grass".into(), from: 1.0, to: 1.5 };
        assert_eq!(describe(&change), "Gather yield: grass: ×1 → ×1.5");
    }

    #[test]
    fn a_refusal_names_each_bad_key() {
        let update = values(&[("property_tax_rate", 0.9), ("nonsense", 1.0)]);
        let e = UpdateError::Refused(vec![
            ("nonsense".into(), TunableError::UnknownKey),
            ("property_tax_rate".into(), TunableError::OutOfRange { min: 0.0, max: 0.5 }),
        ]);
        assert_eq!(e.message(&update), "nonsense: no such tunable; property_tax_rate: 0.9 is outside 0 to 0.5");
    }

    mod db {
        use super::*;

        /// A change is swapped in, saved, annotated and, for a fee, told to the
        /// sockets. Setting it back deletes the row, and the next boot loads
        /// what was saved
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_change_is_live_saved_marked_and_told(db: PgPool) {
            let live = Live::default();
            let (events, mut rx) = broadcast::channel(16);
            let changes = update(&db, &live, &values(&[("listing_fee_percent", 2.5), ("yield.clay", 2.0)]), &known(), &events)
                .await
                .unwrap();
            assert_eq!(changes.len(), 2);
            assert_eq!(live.get().fees.listing_fee_percent, 2.5);
            assert_eq!(live.get().yield_multiplier("clay"), 2.0);
            let Ok(GameEvent::FeesChanged { fees }) = rx.try_recv() else { panic!("no fees_changed") };
            assert_eq!(fees.listing_fee_percent, 2.5);

            let marks = annotations::since(&db, 0, 10).await.unwrap();
            let texts: Vec<_> = marks.iter().map(|m| (m.kind, m.text.as_str())).collect();
            assert_eq!(
                texts,
                [
                    (AnnotationKind::TunableChanged, "Listing fee: 1% → 2.5%"),
                    (AnnotationKind::TunableChanged, "Gather yield: clay: ×1 → ×2"),
                ]
            );
            assert_eq!(load(&db, &known()).await.unwrap(), *live.get());

            // The same values again change nothing, and say nothing
            assert!(update(&db, &live, &values(&[("listing_fee_percent", 2.5)]), &known(), &events).await.unwrap().is_empty());
            assert!(rx.try_recv().is_err());

            // A yield isn't a fee: no fees_changed. Back to its default drops the row
            update(&db, &live, &values(&[("yield.clay", 1.0)]), &known(), &events).await.unwrap();
            assert!(rx.try_recv().is_err());
            let rows: Vec<String> = sqlx::query_scalar("SELECT key FROM tunables").fetch_all(&db).await.unwrap();
            assert_eq!(rows, ["listing_fee_percent"]);
        }

        /// One bad value refuses the whole request: nothing is set, saved or marked
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_bad_value_changes_nothing(db: PgPool) {
            let live = Live::default();
            let (events, mut rx) = broadcast::channel(16);
            let bad = values(&[("stall_rent_per_day", 30.0), ("property_tax_rate", 0.9), ("yield.mithril", 2.0)]);
            let Err(UpdateError::Refused(refused)) = update(&db, &live, &bad, &known(), &events).await else {
                panic!("accepted")
            };
            let keys: Vec<_> = refused.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(keys, ["property_tax_rate", "yield.mithril"]);
            assert_eq!(*live.get(), Tunables::default());
            assert!(rx.try_recv().is_err());
            let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tunables").fetch_one(&db).await.unwrap();
            assert_eq!(rows, 0);
            assert!(annotations::since(&db, 0, 10).await.unwrap().is_empty());
        }

        /// A saved row the code no longer admits is skipped at boot
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_stale_row_loads_as_the_default(db: PgPool) {
            sqlx::query("INSERT INTO tunables (key, value) VALUES ('gone', 3), ('yield.mithril', 2), ('property_tax_rate', 9), ('stall_rent_per_day', 25)")
                .execute(&db)
                .await
                .unwrap();
            let loaded = load(&db, &known()).await.unwrap();
            assert_eq!(loaded.fees.stall_rent_per_day, 25);
            assert_eq!(Tunables { fees: Fees { stall_rent_per_day: 20, ..loaded.fees }, ..loaded }, Tunables::default());
        }

        use capitol_protocol::tunables::Fees;
    }
}

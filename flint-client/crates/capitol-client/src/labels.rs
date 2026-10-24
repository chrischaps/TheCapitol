//! Where the players' names go on the screen (P4.2, moved here in P8.5).
//!
//! Names fade with distance, and shrink a little, so a crowd down the
//! street doesn't read as a wall of text. Yours always comes first and
//! carries further than anyone else's, so you can find yourself from high
//! up. Then the players nearest you win: a name that would overlap one
//! already placed is left out, and past `MAX_NAMES` the rest are too.
//! "Nearest" is to your avatar, not the camera, which looks down from
//! behind you: ranked by the camera, the crowd at the foot of the screen
//! would take every name and your neighbours ahead would have none.
//! Offline there's no you, and the camera's nearest win.
//!
//! `cap_name_tags` projects each name's anchor with Flint's `Projector`
//! (ADR 0087), the camera this frame renders with, and runs [`place`];
//! `hud.rhai` only draws what it returns. The placement was Rhai until
//! P8.5: interpreted, its overlap test was quadratic in the crowd, and at
//! 49 players it took about 2 ms a frame. Here it's microseconds. Until
//! H14 ([#97](https://github.com/chrischaps/TheCapitol/issues/97)) the
//! script projected, and handed a map per player across and back.
//!
//! A citizen's name has a small lozenge before it (C0.2, Citizens D3), in
//! their lantern's pale. The name and the glyph are centred together over
//! the head, and clash as one box. A guild member's tag follows the name in
//! brackets (Commons S4.4), a little smaller and in the dim ink, and is
//! part of the same box: "Ada Reeder [ROPE]".
//!
//! A line someone says (Commons S1.2, plan D3) hangs over their name in a
//! bubble, wrapped to [`WRAP`] characters a row and [`ROWS`] rows, with an
//! ellipsis past that. The bubble is part of the speaker's box, so it's
//! never drawn over another name. A speaker within earshot of you
//! ([`HEARD`]) comes before the silent ones, even past `MAX_NAMES`: in a
//! conversation, the one talking is the one to read. The names are placed
//! first, then the bubbles, newest first: where a bubble would meet a name
//! or another bubble, it slides aside a little if that clears it, or else
//! it's lifted over it. So two people talking side by
//! side both show, and a new line hangs lowest and pushes the older ones
//! up, as a chat does, without moving anyone's name. A speaker whose name
//! has no room (it would clash with one placed before it, yours say, as
//! the row closes up from further off) still gets their bubble, with their
//! name, and a citizen's lozenge, as a caption at its head.
//!
//! [`HEARD`]: capitol_core::protocol::rules::talk::HEARD

use crate::script_api::{PlayerLabel, Speech};

/// Names are whole out to `NEAR` metres from the camera and gone by `FAR`.
pub const NEAR: f32 = 70.0;
pub const FAR: f32 = 170.0;
/// Yours carries further.
pub const MY_NEAR: f32 = 260.0;
pub const MY_FAR: f32 = 420.0;
/// Text size in points, close up and at the fade's end.
pub const SIZE_NEAR: f32 = 15.0;
pub const SIZE_FAR: f32 = 11.0;
/// Room kept between two names, in points.
pub const GAP: f32 = 3.0;
/// The most names shown besides yours. A crowd shows its nearest.
pub const MAX_NAMES: usize = 24;
/// Fainter than this isn't drawn.
const MIN_ALPHA: f32 = 0.02;
/// A citizen's glyph: its half-height, and the room from its right point
/// to the name, as shares of the text size.
pub const GLYPH_R: f32 = 0.24;
pub const GLYPH_GAP: f32 = 0.3;
/// A guild's tag after the name: its size, and the room before it, as
/// shares of the name's size.
pub const TAG_SIZE: f32 = 0.82;
pub const TAG_GAP: f32 = 0.3;
/// A bubble's rows: at most this many characters, and this many rows.
pub const WRAP: usize = 28;
pub const ROWS: usize = 3;
/// A bubble's padding round its text, as shares of the text size.
pub const PAD_X: f32 = 0.55;
pub const PAD_Y: f32 = 0.3;
/// The tail from the bubble down to the name, in points.
pub const TAIL: f32 = 7.0;
/// A caption's size, as a share of the bubble's text.
pub const CAPTION: f32 = 0.8;
/// How far a bubble may slide aside to clear another, as a share of its
/// width. Its tail still points at the speaker.
pub const SLIDE: f32 = 0.35;
/// How many times a bubble is moved to clear another before it gives up.
const TRIES: usize = 8;

/// Where an anchor landed on the screen: logical points, and its depth
/// (distance along the view) in metres. `None` behind the camera.
pub type Screen = Option<[f32; 3]>;

/// A name to draw: the name's top left, in logical points.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub alpha: f32,
    pub me: bool,
    /// A citizen's lozenge, left of the name: its centre and half-height.
    pub glyph: Option<[f32; 3]>,
    /// Their guild's tag after the name (S4.4), bracketed.
    pub tag: Option<GuildTag>,
    /// The name is drawn. False for a speaker whose name had no room: it's
    /// their bubble's caption instead.
    pub named: bool,
    /// What they're saying, over the name.
    pub bubble: Option<Bubble>,
}

/// A guild's tag after a name: the text, its top left and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct GuildTag {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// A line over a speaker's head: a box, its rows, and its tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    /// The box's top left, width and height, in logical points.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The text, a row at a time.
    pub rows: Vec<Row>,
    pub size: f32,
    pub alpha: f32,
    /// Where the tail points: the middle of the name's top, a hair above,
    /// or over the head when the name had no room.
    pub tip: [f32; 2],
    /// Who's speaking, at the bubble's head, when their name had no room.
    pub caption: Option<Caption>,
}

/// A speaker's name in their bubble: its top left, its size, and a
/// citizen's lozenge (centre and half-height) before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Caption {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub glyph: Option<[f32; 3]>,
}

/// A row of a bubble: its text, its centre and its top.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub text: String,
    pub x: f32,
    pub y: f32,
}

/// `text` in rows of at most `width` characters, broken between words (or
/// inside one too long for a row), and at most `rows` of them: past that,
/// the last row ends in an ellipsis.
pub fn wrap(text: &str, width: usize, rows: usize) -> Vec<String> {
    let mut out: Vec<Vec<char>> = Vec::new();
    let mut row: Vec<char> = Vec::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let need = if row.is_empty() {
                word.len()
            } else {
                row.len() + 1 + word.len()
            };
            if need <= width {
                if !row.is_empty() {
                    row.push(' ');
                }
                row.append(&mut word);
                break;
            }
            if !row.is_empty() {
                out.push(std::mem::take(&mut row));
                continue;
            }
            // Longer than a row on its own: break it.
            let rest = word.split_off(width);
            out.push(std::mem::replace(&mut word, rest));
        }
    }
    if !row.is_empty() {
        out.push(row);
    }
    if out.len() > rows {
        out.truncate(rows);
        if let Some(last) = out.last_mut() {
            last.truncate(width.saturating_sub(1));
            while last.last().is_some_and(|c| c.is_whitespace()) {
                last.pop();
            }
            last.push('…');
        }
    }
    out.into_iter().map(|r| r.into_iter().collect()).collect()
}

/// The names to draw, in drawing order (yours, then nearest you first).
/// `screens[i]` is where `players[i]`'s anchor landed. `measure` gives a
/// name's (width, height) at a size. `view` is the screen's size, and a
/// name wholly off it is skipped.
pub fn place(
    players: &[PlayerLabel],
    screens: &[Screen],
    view: [f32; 2],
    mut measure: impl FnMut(&str, f32) -> (f32, f32),
) -> Vec<Tag> {
    struct Candidate<'a> {
        p: &'a PlayerLabel,
        at: [f32; 2],
        depth: f32,
        alpha: f32,
        rank: f32,
        /// Saying something within earshot of you (anywhere, offline).
        speaking: bool,
    }
    let me = players.iter().find(|p| p.me).map(|p| p.at);
    let earshot = capitol_core::protocol::rules::talk::HEARD as f32 * crate::coords::S;
    let mut candidates: Vec<Candidate> = players
        .iter()
        .zip(screens)
        .filter_map(|(p, s)| {
            let [x, y, depth] = (*s)?;
            let (near, far) = if p.me { (MY_NEAR, MY_FAR) } else { (NEAR, FAR) };
            let alpha = (1.0 - smooth((depth - near) / (far - near))) * p.alpha;
            let (rank, heard) = match me {
                Some(m) => {
                    let d = (p.at[0] - m[0]).hypot(p.at[2] - m[2]);
                    (d, d <= earshot)
                }
                None => (depth, true),
            };
            (alpha >= MIN_ALPHA).then_some(Candidate {
                p,
                at: [x, y],
                depth,
                alpha,
                rank,
                speaking: heard && p.line.is_some(),
            })
        })
        .collect();
    // You first, then whoever is talking near you, then nearest first:
    // those are the names that win a clash.
    candidates.sort_by(|a, b| {
        b.p.me
            .cmp(&a.p.me)
            .then(b.speaking.cmp(&a.speaking))
            .then(a.rank.total_cmp(&b.rank))
    });

    let mut placed: Vec<Tag> = Vec::new();
    // Each placed name's box, [x, y, w, h], for the clash test.
    let mut boxes: Vec<[f32; 4]> = Vec::new();
    // The lines to hang once every name is placed.
    struct Said<'a> {
        tag: usize,
        line: &'a Speech,
        alpha: f32,
        /// The middle of the top of the name, or the head, it hangs over.
        over: [f32; 2],
        /// When the name goes in it: whether they're a citizen.
        caption: Option<bool>,
    }
    let mut said: Vec<Said> = Vec::new();
    let mut others = 0;
    for c in candidates {
        if !c.p.me && !c.speaking && others >= MAX_NAMES {
            break;
        }
        let size = lerp(SIZE_NEAR, SIZE_FAR, smooth((c.depth - 20.0) / (FAR - 20.0)));
        let (name_w, h) = measure(&c.p.name, size);
        // The glyph's width and the gap after it, ahead of the name.
        let lead = if c.p.citizen {
            (2.0 * GLYPH_R + GLYPH_GAP) * size
        } else {
            0.0
        };
        // The tag's text, size and width, after the name.
        let tag = c.p.tag.as_deref().map(|t| {
            let text = format!("[{t}]");
            let size = size * TAG_SIZE;
            let (w, th) = measure(&text, size);
            (text, size, w, th)
        });
        let trail = tag.as_ref().map_or(0.0, |t| TAG_GAP * size + t.2);
        let w = lead + name_w + trail;
        let (x, y) = (c.at[0] - w / 2.0, c.at[1] - h);
        if x + w < 0.0 || x > view[0] || y + h < 0.0 || y > view[1] {
            continue;
        }
        let named = clashes(&boxes, [x, y, w, h]).is_none();
        let line = c.p.line.as_ref().filter(|_| c.speaking);
        if !named && line.is_none() {
            continue;
        }
        // Those talking near you are over and above the rest.
        others += usize::from(!c.p.me && !c.speaking);
        if named {
            boxes.push([x, y, w, h]);
        }
        // Level with the middle of the lower-case letters.
        let r = GLYPH_R * size;
        let glyph = c.p.citizen.then_some([x + r, y + h * 0.55, r]);
        if let Some(line) = c.p.line.as_ref() {
            let over = if named { [c.at[0], y] } else { c.at };
            said.push(Said {
                tag: placed.len(),
                line,
                alpha: line.alpha * c.alpha,
                over,
                caption: (!named).then_some(c.p.citizen),
            });
        }
        // On the name's baseline: the smaller text sits low by the difference
        let tag = tag.filter(|_| named).map(|(text, tag_size, _, th)| GuildTag {
            text,
            x: x + lead + name_w + TAG_GAP * size,
            y: y + (h - th) * 0.8,
            size: tag_size,
        });
        placed.push(Tag {
            name: c.p.name.clone(),
            x: x + lead,
            y,
            size,
            alpha: c.alpha,
            me: c.p.me,
            glyph: glyph.filter(|_| named),
            tag,
            named,
            bubble: None,
        });
    }
    // Newest first, so it hangs lowest.
    said.sort_by(|a, b| a.line.age.total_cmp(&b.line.age));
    for s in said {
        let t = &placed[s.tag];
        let caption = s.caption.map(|c| (t.name.as_str(), c));
        let b = bubble(&s.line.text, s.alpha, t.size, s.over, caption, &mut boxes, &mut measure);
        placed[s.tag].bubble = b;
    }
    // A speaker whose name had no room and whose bubble found none either
    // has nothing to show.
    placed.retain(|t| t.named || t.bubble.is_some());
    placed
}

/// The first of `boxes` that `b` comes within `GAP` of.
fn clashes(boxes: &[[f32; 4]], [x, y, w, h]: [f32; 4]) -> Option<[f32; 4]> {
    boxes.iter().copied().find(|&[ox, oy, ow, oh]| {
        x < ox + ow + GAP && ox < x + w + GAP && y < oy + oh + GAP && oy < y + h + GAP
    })
}

/// Lay out `text` in a bubble over the name whose top middle is `over`,
/// lifted over any box in its way, and add its box to `boxes`. `None` when
/// there's nothing to say, or nowhere clear to say it.
fn bubble(
    text: &str,
    alpha: f32,
    size: f32,
    over: [f32; 2],
    caption: Option<(&str, bool)>,
    boxes: &mut Vec<[f32; 4]>,
    measure: &mut impl FnMut(&str, f32) -> (f32, f32),
) -> Option<Bubble> {
    if alpha < MIN_ALPHA {
        return None;
    }
    let rows = wrap(text, WRAP, ROWS);
    if rows.is_empty() {
        return None;
    }
    let measured: Vec<(f32, f32)> = rows.iter().map(|r| measure(r, size)).collect();
    let row_h = measured.iter().map(|m| m.1).fold(0.0, f32::max);
    let (pad_x, pad_y) = (PAD_X * size, PAD_Y * size);
    // The caption's size, its glyph's room, its width and its height.
    let cap = caption.map(|(name, citizen)| {
        let size = size * CAPTION;
        let lead = if citizen { (2.0 * GLYPH_R + GLYPH_GAP) * size } else { 0.0 };
        let (w, h) = measure(name, size);
        (name, citizen, size, lead, lead + w, h)
    });
    let cap_w = cap.map_or(0.0, |c| c.4);
    let cap_h = cap.map_or(0.0, |c| c.5);
    let w = measured.iter().map(|m| m.0).fold(cap_w, f32::max) + 2.0 * pad_x;
    let h = cap_h + row_h * rows.len() as f32 + 2.0 * pad_y;
    let mut x = over[0] - w / 2.0;
    let mut y = over[1] - TAIL - h;
    for _ in 0..TRIES {
        if let Some([ox, oy, ow, _]) = clashes(boxes, [x, y, w, h]) {
            // Aside, the shorter way, if that's near enough and clear...
            let right = ox + ow + GAP + 1.0 - x;
            let left = ox - GAP - 1.0 - (x + w);
            let aside = x + if right.abs() < left.abs() { right } else { left };
            let near = (aside + w / 2.0 - over[0]).abs() <= SLIDE * w;
            if near && clashes(boxes, [aside, y, w, h]).is_none() {
                x = aside;
            } else {
                // ...or up over it, centred again.
                (x, y) = (over[0] - w / 2.0, oy - GAP - 1.0 - h);
            }
            continue;
        }
        boxes.push([x, y, w, h]);
        let middle = x + w / 2.0;
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(i, text)| Row {
                text,
                x: middle,
                y: y + pad_y + cap_h + row_h * i as f32,
            })
            .collect();
        let caption = cap.map(|(name, citizen, size, lead, w, h)| {
            let (left, top) = (middle - w / 2.0, y + pad_y);
            let r = GLYPH_R * size;
            Caption {
                text: name.to_owned(),
                x: left + lead,
                y: top,
                size,
                glyph: citizen.then_some([left + r, top + h * 0.55, r]),
            }
        });
        return Some(Bubble {
            x,
            y,
            w,
            h,
            rows,
            size,
            alpha,
            tip: [over[0], over[1] - 1.0],
            caption,
        });
    }
    None
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: [f32; 2] = [1920.0, 1080.0];

    fn player(name: &str, me: bool) -> PlayerLabel {
        PlayerLabel {
            id: uuid::Uuid::nil(),
            name: name.into(),
            at: [0.0; 3],
            feet_y: 0.0,
            alpha: 1.0,
            me,
            citizen: false,
            tag: None,
            craft: None,
            line: None,
        }
    }

    fn citizen(name: &str) -> PlayerLabel {
        PlayerLabel {
            citizen: true,
            ..player(name, false)
        }
    }

    fn standing(name: &str, me: bool, x: f32, z: f32) -> PlayerLabel {
        PlayerLabel {
            at: [x, 2.0, z],
            ..player(name, me)
        }
    }

    /// Every character 0.6 of the size wide, like Flint's fallback.
    fn measure(text: &str, size: f32) -> (f32, f32) {
        (text.chars().count() as f32 * size * 0.6, size)
    }

    /// The names drawn over heads, not in bubbles.
    fn names(tags: &[Tag]) -> Vec<&str> {
        tags.iter().filter(|t| t.named).map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn a_name_hangs_centred_over_its_anchor() {
        let tags = place(
            &[player("Ada", false)],
            &[Some([500.0, 300.0, 30.0])],
            VIEW,
            measure,
        );
        let t = &tags[0];
        let (w, h) = measure("Ada", t.size);
        assert_eq!((t.x + w / 2.0, t.y + h), (500.0, 300.0));
        assert_eq!(t.alpha, 1.0);
    }

    #[test]
    fn a_members_tag_follows_the_name_and_the_two_are_centred_as_one() {
        let ada = PlayerLabel { tag: Some("ROPE".into()), ..standing("Ada", false, 0.0, -10.0) };
        let tags = place(&[ada], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        let t = &tags[0];
        let g = t.tag.as_ref().expect("a tag");
        assert_eq!(g.text, "[ROPE]");
        let (name_w, _) = measure("Ada", t.size);
        let (tag_w, _) = measure("[ROPE]", g.size);
        assert!(g.size < t.size, "smaller than the name");
        assert!((g.x - (t.x + name_w + TAG_GAP * t.size)).abs() < 0.01, "after the name");
        // Name, gap and tag centred over the anchor
        let middle = (t.x + g.x + tag_w) / 2.0;
        assert!((middle - 500.0).abs() < 0.01, "{middle}");
        // No tag without a guild
        let tags = place(&[standing("Bram", false, 0.0, -10.0)], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        assert_eq!(tags[0].tag, None);
    }

    #[test]
    fn a_tag_is_part_of_the_names_box_in_a_clash() {
        // Bram's name clears Ada's alone; her tag reaches him
        let ada = PlayerLabel { tag: Some("ROPE".into()), ..standing("Ada", false, 0.0, -10.0) };
        let bram = standing("Bram", false, 0.0, -11.0);
        let size = SIZE_NEAR;
        let ada_w = measure("Ada", size).0;
        let at_ada = [500.0, 300.0, 10.0];
        let at_bram = [500.0 + ada_w / 2.0 + GAP + 2.0 + measure("Bram", size).0 / 2.0, 300.0, 11.0];
        let alone = place(&[standing("Ada", false, 0.0, -10.0), bram.clone()], &[Some(at_ada), Some(at_bram)], VIEW, measure);
        assert_eq!(alone.len(), 2);
        let tagged = place(&[ada, bram], &[Some(at_ada), Some(at_bram)], VIEW, measure);
        assert_eq!(tagged.len(), 1, "the nearer, tagged name wins");
    }

    #[test]
    fn a_citizens_name_and_glyph_hang_centred_together_and_clash_as_one() {
        let tags = place(
            &[citizen("Corin")],
            &[Some([500.0, 300.0, 30.0])],
            VIEW,
            measure,
        );
        let t = &tags[0];
        let [gx, gy, r] = t.glyph.expect("a glyph");
        let (w, h) = measure("Corin", t.size);
        // The glyph, the gap, then the name, centred as one over the anchor.
        assert!(
            (gx - r + (t.x + w) - 1000.0).abs() < 1e-3,
            "{gx} {r} {}",
            t.x
        );
        assert!(t.x - (gx + r) > 0.0, "a gap before the name");
        assert!(gy > t.y && gy < t.y + h, "beside the letters");
        // A person's name has none.
        let tags = place(
            &[player("Wren", false)],
            &[Some([500.0, 300.0, 30.0])],
            VIEW,
            measure,
        );
        assert_eq!(tags[0].glyph, None);
        // The glyph is part of the name's box: a name just clear of the
        // citizen's text but over its glyph loses.
        let lead = (2.0 * GLYPH_R + GLYPH_GAP) * t.size;
        let (bw, _) = measure("Bo", t.size);
        let left = gx - r - GAP - 1.0 - bw / 2.0;
        let players = [citizen("Corin"), player("Bo", false)];
        let screens = [
            Some([500.0, 300.0, 30.0]),
            Some([left + lead / 2.0, 300.0, 30.0]),
        ];
        assert_eq!(place(&players, &screens, VIEW, measure).len(), 1);
        let screens = [Some([500.0, 300.0, 30.0]), Some([left, 300.0, 30.0])];
        assert_eq!(place(&players, &screens, VIEW, measure).len(), 2);
    }

    #[test]
    fn names_fade_and_shrink_with_distance_and_yours_carries_further() {
        let at = |d| {
            place(
                &[player("Ada", false)],
                &[Some([500.0, 300.0, d])],
                VIEW,
                measure,
            )
        };
        let mine = |d| {
            place(
                &[player("Me", true)],
                &[Some([500.0, 300.0, d])],
                VIEW,
                measure,
            )
        };
        assert_eq!(at(NEAR)[0].alpha, 1.0);
        let mid = &at((NEAR + FAR) / 2.0)[0];
        assert!(mid.alpha > 0.4 && mid.alpha < 0.6, "{}", mid.alpha);
        assert!(mid.size < SIZE_NEAR && mid.size > SIZE_FAR);
        assert!(at(FAR).is_empty());
        assert_eq!(mine(FAR)[0].alpha, 1.0);
        assert!(mine(MY_FAR).is_empty());
    }

    #[test]
    fn a_player_fading_in_takes_its_name_with_it() {
        let mut p = player("Ada", false);
        p.alpha = 0.5;
        let tags = place(&[p.clone()], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        assert_eq!(tags[0].alpha, 0.5);
        p.alpha = 0.0;
        assert!(place(&[p], &[Some([500.0, 300.0, 10.0])], VIEW, measure).is_empty());
    }

    #[test]
    fn nearer_names_win_a_clash_and_yours_beats_them_all() {
        let players = [
            standing("Far", false, 0.0, -20.0),
            standing("Near", false, 0.0, -5.0),
            standing("Me", true, 0.0, 0.0),
        ];
        // All three on the same spot, you the farthest.
        let screens = [
            Some([500.0, 300.0, 40.0]),
            Some([500.0, 300.0, 20.0]),
            Some([500.0, 300.0, 60.0]),
        ];
        assert_eq!(names(&place(&players, &screens, VIEW, measure)), ["Me"]);
        // Without you, the nearer of the other two.
        assert_eq!(
            names(&place(&players[..2], &screens[..2], VIEW, measure)),
            ["Near"]
        );
        // Apart, all three, you first and then nearest first.
        let apart = [
            Some([100.0, 300.0, 40.0]),
            Some([500.0, 300.0, 20.0]),
            Some([900.0, 300.0, 60.0]),
        ];
        assert_eq!(
            names(&place(&players, &apart, VIEW, measure)),
            ["Me", "Near", "Far"]
        );
    }

    #[test]
    fn signed_in_the_names_nearest_you_win_not_those_nearest_the_camera() {
        // You at the origin. Ahead stands 3 m beyond you, Behind 12 m
        // short of you, nearer the camera.
        let players = [
            standing("Behind", false, 0.0, 12.0),
            standing("Ahead", false, 0.0, -3.0),
            standing("Me", true, 0.0, 0.0),
        ];
        // Ahead and Behind on the same spot on the screen.
        let screens = [
            Some([500.0, 300.0, 18.0]),
            Some([500.0, 300.0, 33.0]),
            Some([900.0, 300.0, 30.0]),
        ];
        assert_eq!(
            names(&place(&players, &screens, VIEW, measure)),
            ["Me", "Ahead"]
        );
    }

    #[test]
    fn names_a_gap_apart_both_stand() {
        let players = [player("Ada", false), player("Bo", false)];
        let (w, _) = measure("Ada", SIZE_NEAR);
        let x = 500.0 + w / 2.0 + GAP + 1.0 + measure("Bo", SIZE_NEAR).0 / 2.0;
        let screens = [Some([500.0, 300.0, 10.0]), Some([x, 300.0, 10.0])];
        assert_eq!(place(&players, &screens, VIEW, measure).len(), 2);
        let screens = [Some([500.0, 300.0, 10.0]), Some([x - 2.0, 300.0, 10.0])];
        assert_eq!(place(&players, &screens, VIEW, measure).len(), 1);
    }

    #[test]
    fn a_crowd_shows_only_its_nearest_names_but_always_yours() {
        let mut players: Vec<_> = (0..60)
            .map(|i| standing(&format!("P{i}"), false, 100.0 - i as f32, 0.0))
            .collect();
        players.push(standing("Me", true, 0.0, 0.0));
        // A grid of names far enough apart never to clash, nearest last,
        // and you the farthest of all.
        let mut screens: Vec<_> = (0..60)
            .map(|i| {
                Some([
                    60.0 + (i % 10) as f32 * 180.0,
                    60.0 + (i / 10) as f32 * 150.0,
                    100.0 - i as f32,
                ])
            })
            .collect();
        screens.push(Some([960.0, 1040.0, 150.0]));
        let tags = place(&players, &screens, VIEW, measure);
        assert_eq!(tags.len(), MAX_NAMES + 1);
        assert_eq!(tags[0].name, "Me");
        let expect: Vec<String> = (36..60).rev().map(|i| format!("P{i}")).collect();
        assert_eq!(names(&tags[1..]), expect);
    }

    #[test]
    fn names_behind_the_camera_or_off_the_screen_are_left_out_and_dont_crowd_others() {
        let players = [
            player("Behind", false),
            player("Left", false),
            player("Below", false),
            player("Seen", false),
        ];
        let screens = [
            None,
            Some([-200.0, 300.0, 5.0]),
            Some([500.0, 1200.0, 5.0]),
            Some([500.0, 300.0, 60.0]),
        ];
        assert_eq!(names(&place(&players, &screens, VIEW, measure)), ["Seen"]);
    }

    #[test]
    fn a_hundred_and_twenty_players_place_quickly() {
        let players: Vec<_> = (0..120)
            .map(|i| player(&format!("Perfcrowd{i}"), false))
            .collect();
        let screens: Vec<_> = (0..120)
            .map(|i| {
                Some([
                    (i * 97 % 1900) as f32,
                    (i * 53 % 1060) as f32 + 20.0,
                    10.0 + i as f32,
                ])
            })
            .collect();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(place(&players, &screens, VIEW, measure));
        }
        let each = start.elapsed() / 100;
        // In Rhai this took milliseconds. Debug builds are slow; still far under.
        assert!(each.as_micros() < 2000, "{each:?}");
    }

    fn saying(p: PlayerLabel, text: &str) -> PlayerLabel {
        said(p, text, 0.0)
    }

    fn said(p: PlayerLabel, text: &str, age: f32) -> PlayerLabel {
        PlayerLabel {
            line: Some(Speech {
                text: text.into(),
                alpha: 1.0,
                age,
            }),
            ..p
        }
    }

    #[test]
    fn a_line_wraps_to_28_characters_a_row_between_words() {
        assert_eq!(
            wrap("Has anyone seen the price of iron this week?", WRAP, ROWS),
            ["Has anyone seen the price of", "iron this week?"]
        );
        assert_eq!(wrap("Morning.", WRAP, ROWS), ["Morning."]);
        // A word too long for a row is broken inside.
        let long = "x".repeat(60);
        assert_eq!(
            wrap(&long, WRAP, ROWS),
            ["x".repeat(28), "x".repeat(28), "x".repeat(4)]
        );
        // Nothing to say, no rows.
        assert!(wrap("   ", WRAP, ROWS).is_empty());
        let text = "The kiln's lit if you need bricks, and the forge will be by noon.";
        for row in wrap(text, WRAP, ROWS) {
            assert!(row.chars().count() <= WRAP, "{row}");
        }
    }

    #[test]
    fn past_three_rows_the_last_ends_in_an_ellipsis() {
        let rows = wrap(&["word"; 20].join(" "), WRAP, ROWS);
        assert_eq!(rows.len(), ROWS);
        assert_eq!(rows[2], "word word word word word…");
        // A full last row gives up its last character for it.
        let rows = wrap(&"x".repeat(100), WRAP, ROWS);
        assert_eq!(rows[2], format!("{}…", "x".repeat(27)));
        assert_eq!(rows[2].chars().count(), WRAP);
        // Three rows exactly need none.
        let rows = wrap(&"x".repeat(84), WRAP, ROWS);
        assert!(rows.iter().all(|r| !r.ends_with('…')));
    }

    #[test]
    fn a_bubble_hangs_over_its_speakers_name() {
        let tags = place(
            &[saying(player("Ada", false), "Hello there")],
            &[Some([500.0, 300.0, 10.0])],
            VIEW,
            measure,
        );
        let t = &tags[0];
        let b = t.bubble.as_ref().expect("a bubble");
        assert_eq!(b.rows.len(), 1);
        assert_eq!(b.rows[0].text, "Hello there");
        assert!((b.x + b.w / 2.0 - 500.0).abs() < 1e-3, "centred");
        assert!((b.y + b.h + TAIL - t.y).abs() < 1e-3, "a tail over the name");
        assert_eq!(b.tip[0], 500.0);
        let (w, _) = measure("Hello there", t.size);
        assert!(b.w > w && b.h > t.size, "padded");
        // Silent, none.
        let tags = place(&[player("Ada", false)], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        assert_eq!(tags[0].bubble, None);
    }

    #[test]
    fn a_bubble_is_lifted_clear_of_a_name_and_never_drawn_over_one() {
        // Bo's name sits where Ada's bubble would go.
        let screens = [Some([500.0, 300.0, 10.0]), Some([500.0, 270.0, 8.0])];
        let silent = [player("Ada", false), player("Bo", false)];
        assert_eq!(names(&place(&silent, &screens, VIEW, measure)), ["Bo", "Ada"]);
        // Talking, Ada comes first, and her bubble is lifted over Bo's name.
        let talking = [saying(player("Ada", false), "Hello there"), player("Bo", false)];
        let tags = place(&talking, &screens, VIEW, measure);
        assert_eq!(names(&tags), ["Ada", "Bo"]);
        let (b, bo) = (tags[0].bubble.as_ref().unwrap(), &tags[1]);
        assert!(b.y + b.h + GAP < bo.y, "{} {}", b.y + b.h, bo.y);
        assert_eq!(b.tip[0], 500.0, "its tail still at Ada");
    }

    #[test]
    fn two_people_talking_side_by_side_both_show_one_bubble_lifted_over_the_other() {
        let (aw, _) = measure("Ada", SIZE_NEAR);
        let (bw, _) = measure("Bo", SIZE_NEAR);
        let bx = 500.0 + aw / 2.0 + GAP + 4.0 + bw / 2.0;
        let players = [
            saying(player("Ada", false), "Hello there"),
            saying(player("Bo", false), "And to you"),
        ];
        let screens = [Some([500.0, 300.0, 10.0]), Some([bx, 300.0, 10.0])];
        let tags = place(&players, &screens, VIEW, measure);
        assert_eq!(names(&tags), ["Ada", "Bo"]);
        let a = tags[0].bubble.as_ref().unwrap();
        let b = tags[1].bubble.as_ref().unwrap();
        assert!(b.y + b.h + GAP < a.y, "Bo's over Ada's: {} {}", b.y + b.h, a.y);
        // Its tail still points at Bo.
        assert_eq!(b.tip[0], bx);
    }

    #[test]
    fn a_bubble_just_overlapping_another_slides_aside_rather_than_up() {
        let players = [
            saying(player("Ada", false), "Hello there"),
            saying(player("Bo", false), "And to you"),
        ];
        let screens = [Some([500.0, 300.0, 10.0]), Some([600.0, 300.0, 10.0])];
        let tags = place(&players, &screens, VIEW, measure);
        let a = tags[0].bubble.as_ref().unwrap();
        let b = tags[1].bubble.as_ref().unwrap();
        assert_eq!(a.y, b.y, "level");
        assert!(b.x >= a.x + a.w + GAP, "beside it: {} {}", b.x, a.x + a.w);
        // The tail still points at Bo, from under the slip.
        assert_eq!(b.tip[0], 600.0);
        assert!(b.tip[0] > b.x && b.tip[0] < b.x + b.w);
        // The rows follow the slip.
        assert_eq!(b.rows[0].x, b.x + b.w / 2.0);
    }

    #[test]
    fn the_newest_line_hangs_lowest_and_pushes_the_older_up() {
        let (aw, _) = measure("Ada", SIZE_NEAR);
        let (bw, _) = measure("Bo", SIZE_NEAR);
        let bx = 500.0 + aw / 2.0 + GAP + 4.0 + bw / 2.0;
        let screens = [Some([500.0, 300.0, 10.0]), Some([bx, 300.0, 10.0])];
        let lowest = |ada: f32, bo: f32| {
            let players = [
                said(player("Ada", false), "Hello there", ada),
                said(player("Bo", false), "And to you", bo),
            ];
            let tags = place(&players, &screens, VIEW, measure);
            let (a, b) = (tags[0].bubble.clone().unwrap(), tags[1].bubble.clone().unwrap());
            if a.y > b.y { "Ada" } else { "Bo" }
        };
        assert_eq!(lowest(0.5, 3.0), "Ada");
        assert_eq!(lowest(3.0, 0.5), "Bo");
    }

    #[test]
    fn a_speaker_whose_name_has_no_room_still_talks_and_is_named_in_the_bubble() {
        // Corin's name would clash with yours, close up beside it.
        let players = [
            standing("Me", true, 0.0, 0.0),
            saying(standing("Corin", false, 1.6, 0.0), "Morning."),
        ];
        let players = [players[0].clone(), PlayerLabel { citizen: true, ..players[1].clone() }];
        let screens = [Some([500.0, 300.0, 10.0]), Some([520.0, 300.0, 10.0])];
        let tags = place(&players, &screens, VIEW, measure);
        assert_eq!(names(&tags), ["Me"]);
        let corin = tags.iter().find(|t| t.name == "Corin").expect("still there");
        assert!(!corin.named && corin.glyph.is_none(), "no name over the head");
        let b = corin.bubble.as_ref().expect("a bubble");
        let c = b.caption.as_ref().expect("named in it");
        assert_eq!(c.text, "Corin");
        assert!(c.glyph.is_some(), "a citizen's mark goes with the name");
        assert!(c.y >= b.y && b.rows[0].y > c.y, "the caption heads the bubble");
        assert!(c.size < b.size);
        assert_eq!(b.tip, [520.0, 299.0], "the tail to the head");
        // Clear of your name.
        let me = &tags[0];
        assert!(b.y + b.h + GAP < me.y || b.y > me.y + me.size);
        // Silent, a name with no room is simply left out.
        let quiet = [players[0].clone(), standing("Corin", false, 1.6, 0.0)];
        assert_eq!(place(&quiet, &screens, VIEW, measure).len(), 1);
    }

    #[test]
    fn a_citizens_bubble_hangs_over_the_glyph_and_name_together() {
        let tags = place(
            &[saying(citizen("Corin"), "Morning.")],
            &[Some([500.0, 300.0, 10.0])],
            VIEW,
            measure,
        );
        let b = tags[0].bubble.as_ref().unwrap();
        assert!((b.x + b.w / 2.0 - 500.0).abs() < 1e-3, "{}", b.x + b.w / 2.0);
    }

    #[test]
    fn a_speaker_in_earshot_is_kept_past_max_names_and_one_out_of_it_isnt() {
        let earshot = capitol_core::protocol::rules::talk::HEARD as f32 * crate::coords::S;
        let mut players: Vec<_> = (0..40)
            .map(|i| standing(&format!("P{i}"), false, 0.01 * i as f32, 0.0))
            .collect();
        players.push(saying(standing("Near", false, earshot - 0.1, 0.0), "Over here"));
        players.push(saying(standing("Far", false, earshot + 2.0, 0.0), "Can you hear me?"));
        players.push(standing("Me", true, 0.0, 0.0));
        // A grid far enough apart that nothing clashes, bubbles and all.
        let screens: Vec<_> = (0..players.len())
            .map(|i| {
                Some([
                    100.0 + (i % 6) as f32 * 300.0,
                    100.0 + (i / 6) as f32 * 120.0,
                    10.0,
                ])
            })
            .collect();
        let view = [1920.0, 1200.0];
        let tags = place(&players, &screens, view, measure);
        let shown = names(&tags);
        assert_eq!(shown[..2], ["Me", "Near"], "you, then the one talking");
        assert!(!shown.contains(&"Far"));
        assert_eq!(tags.len(), MAX_NAMES + 2);
        assert!(tags[1].bubble.is_some());
    }

    #[test]
    fn a_fading_line_takes_its_bubble_with_it() {
        let mut p = saying(player("Ada", false), "Hello");
        if let Some(line) = &mut p.line {
            line.alpha = 0.5;
        }
        p.alpha = 0.5;
        let tags = place(&[p.clone()], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        assert_eq!(tags[0].bubble.as_ref().unwrap().alpha, 0.25);
        if let Some(line) = &mut p.line {
            line.alpha = 0.0;
        }
        let tags = place(&[p], &[Some([500.0, 300.0, 10.0])], VIEW, measure);
        assert_eq!(tags[0].bubble, None);
    }
}

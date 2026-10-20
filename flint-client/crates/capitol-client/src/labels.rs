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
//! the head, and clash as one box.

use crate::script_api::PlayerLabel;

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
    }
    let me = players.iter().find(|p| p.me).map(|p| p.at);
    let mut candidates: Vec<Candidate> = players
        .iter()
        .zip(screens)
        .filter_map(|(p, s)| {
            let [x, y, depth] = (*s)?;
            let (near, far) = if p.me { (MY_NEAR, MY_FAR) } else { (NEAR, FAR) };
            let alpha = (1.0 - smooth((depth - near) / (far - near))) * p.alpha;
            let rank = match me {
                Some(m) => (p.at[0] - m[0]).hypot(p.at[2] - m[2]),
                None => depth,
            };
            (alpha >= MIN_ALPHA).then_some(Candidate {
                p,
                at: [x, y],
                depth,
                alpha,
                rank,
            })
        })
        .collect();
    // You first, then nearest first: those are the names that win a clash.
    candidates.sort_by(|a, b| b.p.me.cmp(&a.p.me).then(a.rank.total_cmp(&b.rank)));

    let mut placed: Vec<Tag> = Vec::new();
    // Each placed name's box, [x, y, w, h], for the clash test.
    let mut boxes: Vec<[f32; 4]> = Vec::new();
    let mut others = 0;
    for c in candidates {
        if !c.p.me && others >= MAX_NAMES {
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
        let w = lead + name_w;
        let (x, y) = (c.at[0] - w / 2.0, c.at[1] - h);
        if x + w < 0.0 || x > view[0] || y + h < 0.0 || y > view[1] {
            continue;
        }
        let clash = boxes.iter().any(|&[ox, oy, ow, oh]| {
            x < ox + ow + GAP && ox < x + w + GAP && y < oy + oh + GAP && oy < y + h + GAP
        });
        if clash {
            continue;
        }
        others += usize::from(!c.p.me);
        boxes.push([x, y, w, h]);
        // Level with the middle of the lower-case letters.
        let r = GLYPH_R * size;
        let glyph = c.p.citizen.then_some([x + r, y + h * 0.55, r]);
        placed.push(Tag {
            name: c.p.name.clone(),
            x: x + lead,
            y,
            size,
            alpha: c.alpha,
            me: c.p.me,
            glyph,
        });
    }
    placed
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
            craft: None,
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

    fn names(tags: &[Tag]) -> Vec<&str> {
        tags.iter().map(|t| t.name.as_str()).collect()
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
}

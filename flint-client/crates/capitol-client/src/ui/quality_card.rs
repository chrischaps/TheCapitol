//! The quality card (#443, `the-capitol-road-in.md` §4.1): one card on the
//! road in, between twisting the first rope and making the bench kit, on
//! what a craft's quality comes from. It's the cascade (GDD §6) in words,
//! and in the player's own numbers, not made-up ones: the rope they
//! twisted, and the two ropes their worst and their best fiber would
//! twist to (`onboarding::QualityLesson`), as the crafting panel's preview
//! (`crafting::preview`) would show them. Then a line on maker's marks.
//!
//! `hints` shows it in the bench kit's card's place, once a session, until
//! "Go on". `--hints-sample make_workbench` shows it on a sample pack.

use capitol_core::catalog::Catalog;
use capitol_core::onboarding::QualityLesson;
use capitol_core::protocol::rest::SlotItem;
use capitol_core::protocol::rules::stacking::Units;
use egui::RichText;
use uuid::Uuid;

use super::theme::{self, size};

/// The card's title.
pub const TITLE: &str = "What goes in shows";

/// What the cascade is, in a line or two.
pub const WORDS: &str = "A thing is as good as what went into it, give or take a small roll. \
     Crafting's preview shows the range before you start, and where it comes from.";

/// Maker's marks (#231), in a line.
const MARK: &str = "Whatever you make carries your mark, so good work is known by its maker.";

/// Draw the card: the chapter, the title, your rope, the two twists, and
/// the words; `compact` leaves the words to a hover. True when "Go on" is
/// clicked.
pub fn show(ui: &mut egui::Ui, lesson: &QualityLesson, compact: bool) -> bool {
    ui.horizontal(|ui| {
        ui.label(theme::label(super::hints::chapter_name(capitol_core::protocol::tutorial::Chapter::Arrival)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(theme::dim("A word on quality").size(size::SMALL));
        });
    });
    ui.add_space(4.0);
    ui.label(theme::title(TITLE, size::HEADING + 2.0));
    ui.add_space(4.0);
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let yours = if lesson.ropes.len() > 1 { "Your ropes came out at" } else { "Your rope came out at" };
            ui.label(theme::text(yours).size(size::SMALL));
            for &q in &lesson.ropes {
                theme::quality_badge(ui, q);
            }
        });
        match (lesson.fiber, lesson.twists) {
            (Some((lo, hi)), Some((worst, best))) if lo != hi => {
                twist(ui, "Two of your worst fiber would twist to", lo, worst);
                twist(ui, "Two of your best, to", hi, best);
            }
            (Some((q, _)), Some((made, _))) => twist(ui, "Two of your fiber would twist to", q, made),
            _ => {
                ui.add_space(2.0);
                ui.label(theme::dim("Near the fiber that went in, give or take the roll.").size(size::SMALL));
            }
        }
    });
    if !compact {
        ui.add_space(4.0);
        ui.label(RichText::new(WORDS).color(theme::TEXT).size(size::BODY));
        ui.add_space(4.0);
        ui.label(theme::dim(MARK).size(size::SMALL));
    }
    ui.add_space(6.0);
    let mut read = false;
    // In a row, so the link's layout takes a line, not what's left of the card
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            read = theme::link(ui, "Go on", size::SMALL)
                .on_hover_text("On to the workbench kit")
                .clicked();
        });
    });
    read
}

/// One twist: what it is, then two units of fiber at `fiber` and the
/// rope's range they'd make.
fn twist(ui: &mut egui::Ui, what: &str, fiber: i32, rope: (i32, i32)) {
    ui.add_space(4.0);
    ui.label(theme::dim(what).size(size::LABEL));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        theme::quality_badge(ui, fiber);
        ui.label(theme::dim("+"));
        theme::quality_badge(ui, fiber);
        ui.label(RichText::new("→").color(theme::GOLD_DIM).size(size::BODY));
        theme::range_badge(ui, rope);
        ui.label(theme::dim("rope").size(size::LABEL));
    });
}

/// The card's numbers on a sample pack, for `--hints-sample`: three stacks
/// of fiber from tufts of three qualities and the rope twisted from them,
/// through the real `QualityLesson::of` and the fallback catalog's rope.
pub fn sample() -> Option<QualityLesson> {
    let item = |kind: &str, units: &[(i32, i32)]| {
        SlotItem::new(Uuid::nil(), kind.into(), kind.into(), 0, 0, 0).with_units(Units::from_pairs(units.iter().copied()))
    };
    let pack = [item("fiber", &[(41, 2), (58, 3)]), item("rope", &[(52, 1)]), item("fiber", &[(66, 2)])];
    let catalog = Catalog::fallback();
    QualityLesson::of(&pack, catalog.recipes().iter().find(|r| r.id == "rope_basic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_has_a_rope_and_two_twists() {
        let lesson = sample().expect("a sample");
        assert_eq!(lesson.ropes, vec![52]);
        assert_eq!(lesson.fiber, Some((41, 66)));
        assert!(lesson.twists.is_some());
    }

    #[test]
    fn the_card_draws_full_and_compact() {
        let ctx = theme::test_context();
        let lesson = sample().unwrap();
        for compact in [false, true] {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!show(ui, &lesson, compact), "nothing clicked");
                });
            });
        }
    }
}

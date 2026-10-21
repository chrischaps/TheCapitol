//! Going to the Exchange hall (E3.7): click the hall and the market panel
//! opens there, or you walk to its door and it opens as you come into
//! range, as a station does (`station::visit`). The range is the server's,
//! [`rules::market::at_the_hall`]: 75 units from the footprint.
//!
//! And [`goods_waiting`]: whether the market box holds anything, for the
//! gold mote over the hall and the header's hint.

use capitol_protocol::rules;
use capitol_protocol::terrain::Landmark;

use crate::state::Container;

/// How far out from the door a walk to the hall stops, in server units: on
/// the pavement in front of it, well inside the range.
pub const APPROACH: f64 = 20.0;

/// Where to walk to reach `hall`: in front of its door, out from the middle.
pub fn approach(hall: &Landmark) -> [f64; 2] {
    let (dx, dy) = (hall.door.0 - hall.position.0, hall.door.1 - hall.position.1);
    let d = dx.hypot(dy).max(1e-6);
    [
        hall.door.0 + dx / d * APPROACH,
        hall.door.1 + dy / d * APPROACH,
    ]
}

/// What a click on the hall, or an arrival, asks of the game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Open the market panel.
    Open,
    /// Walk to this server point; the panel opens on arrival.
    WalkTo { x: f64, y: f64 },
}

/// A walk to the hall, if one is under way.
#[derive(Debug, Default)]
pub struct Visit {
    walking: bool,
}

impl Visit {
    /// The hall was clicked with you standing at `at`.
    pub fn click(&mut self, hall: &Landmark, at: [f64; 2]) -> Step {
        if rules::market::at_the_hall(hall, at[0], at[1]) {
            self.walking = false;
            return Step::Open;
        }
        self.walking = true;
        let [x, y] = approach(hall);
        Step::WalkTo { x, y }
    }

    /// Each frame: true once, as a walk to the hall comes into range.
    pub fn arrived(&mut self, hall: &Landmark, at: [f64; 2]) -> bool {
        if self.walking && rules::market::at_the_hall(hall, at[0], at[1]) {
            self.walking = false;
            return true;
        }
        false
    }

    /// You walked somewhere else: the hall is no longer where you're going.
    pub fn walked(&mut self) {
        self.walking = false;
    }

    pub fn walking(&self) -> bool {
        self.walking
    }
}

/// Whether the market box holds anything to collect.
pub fn goods_waiting(market_box: Option<&Container>) -> bool {
    market_box.is_some_and(|c| c.slots.iter().any(|s| s.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::terrain::LandmarkKind;

    /// E3.4's hall: 80 × 100 on the Capitol avenue's east side, door west.
    fn hall() -> Landmark {
        Landmark {
            id: "exchange_hall".into(),
            kind: LandmarkKind::ExchangeHall,
            position: (6885.0, 7097.5),
            corners: [
                (6845.0, 7147.5),
                (6845.0, 7047.5),
                (6925.0, 7047.5),
                (6925.0, 7147.5),
            ],
            entrance_edge: (0, 1),
            door: (6845.0, 7097.5),
        }
    }

    #[test]
    fn the_approach_is_on_the_pavement_in_front_of_the_door_and_in_range() {
        let h = hall();
        let [x, y] = approach(&h);
        assert_eq!([x, y], [6825.0, 7097.5]);
        assert!(rules::market::at_the_hall(&h, x, y));
    }

    #[test]
    fn a_click_in_range_opens_and_one_from_afar_walks_then_opens_on_arrival() {
        let h = hall();
        let mut v = Visit::default();
        assert_eq!(v.click(&h, [6800.0, 7100.0]), Step::Open);
        assert!(!v.walking());

        let from = [6800.0, 7700.0];
        assert_eq!(
            v.click(&h, from),
            Step::WalkTo {
                x: 6825.0,
                y: 7097.5
            }
        );
        assert!(!v.arrived(&h, [6800.0, 7400.0]), "still on the way");
        assert!(v.arrived(&h, [6810.0, 7190.0]), "in range");
        assert!(!v.arrived(&h, [6810.0, 7180.0]), "only once");
    }

    #[test]
    fn walking_elsewhere_drops_the_visit() {
        let h = hall();
        let mut v = Visit::default();
        v.click(&h, [6800.0, 7700.0]);
        v.walked();
        assert!(!v.arrived(&h, [6825.0, 7097.5]));
    }

    #[test]
    fn goods_wait_when_any_slot_holds_something() {
        use capitol_protocol::rest::SlotItem;
        use uuid::Uuid;
        let id = Uuid::from_u128(1);
        let plank = SlotItem::new(Uuid::from_u128(2), "plank".into(), "Plank".into(), 40, 3, 1);
        assert!(!goods_waiting(None));
        assert!(!goods_waiting(Some(&Container::new(
            id,
            "market_box",
            2,
            2,
            &[]
        ))));
        assert!(goods_waiting(Some(&Container::new(
            id,
            "market_box",
            2,
            2,
            &[plank]
        ))));
    }
}

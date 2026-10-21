//! Annotations (the Observatory's D0.4): the things that explain a bend in a
//! chart. The backend keeps them in `admin_events`, and the console draws each
//! one as a marker on every chart, so a change in the money or the tick sits
//! next to what happened just before it.

use serde::{Deserialize, Serialize};

/// What happened. Stored as its snake_case name, which the table's check
/// lists, so a new kind needs a migration as well as a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    /// The server started
    Boot,
    /// An admin committed new world params, and the plots and nodes followed
    TerrainCommitted,
    /// A day's Gazette came out
    GazettePublished,
    /// A live tunable changed (D6.1)
    TunableChanged,
    /// The town's citizens came on (D4.1)
    TownUp,
    /// The town's citizens went away (D4.1)
    TownDown,
    /// A watch's rule fired (D2.5)
    WatchFired,
}

impl AnnotationKind {
    pub const ALL: [AnnotationKind; 7] = [
        AnnotationKind::Boot,
        AnnotationKind::TerrainCommitted,
        AnnotationKind::GazettePublished,
        AnnotationKind::TunableChanged,
        AnnotationKind::TownUp,
        AnnotationKind::TownDown,
        AnnotationKind::WatchFired,
    ];

    /// The name it's stored and sent under
    pub fn as_str(self) -> &'static str {
        match self {
            AnnotationKind::Boot => "boot",
            AnnotationKind::TerrainCommitted => "terrain_committed",
            AnnotationKind::GazettePublished => "gazette_published",
            AnnotationKind::TunableChanged => "tunable_changed",
            AnnotationKind::TownUp => "town_up",
            AnnotationKind::TownDown => "town_down",
            AnnotationKind::WatchFired => "watch_fired",
        }
    }

    pub fn parse(name: &str) -> Option<AnnotationKind> {
        AnnotationKind::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

/// One marker: when, what, and a plain sentence for the console to show
/// beside it ("The Gazette, No. 12: Rope at a record").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Annotation {
    pub id: i64,
    /// By the server's clock (Unix ms), like a sample's
    pub at_ms: i64,
    pub kind: AnnotationKind,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_is_stored_under_its_serde_name() {
        for kind in AnnotationKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(AnnotationKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(AnnotationKind::parse("nonsense"), None);
    }
}

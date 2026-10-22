use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use capitol_protocol::rest::{CatalogTool, Maker, SlotItem};
pub use capitol_protocol::rest::InventoryItem;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::tools;

/// The `item_types` columns an [`ItemType`] reads, for `SELECT`s.
pub const ITEM_TYPE_COLUMNS: &str =
    "id, name, category, stackable, weight, tool_type, tier, base_power, max_durability, target_price";

/// A stack's maker as SQL (#242): `json` `{name, citizen}` for the player whose
/// mark every unit of stack `$item` carries, NULL when a unit has none, the
/// marks are mixed, or the maker's byline is off (`players.gazette_named`).
macro_rules! stack_maker_sql {
    ($item:literal) => {
        concat!(
            "(SELECT json_build_object('name', p.name, 'citizen', COALESCE(a.kind = 'citizen', FALSE))
               FROM players p LEFT JOIN accounts a ON a.id = p.account_id
              WHERE p.gazette_named AND p.id = (
                    SELECT (array_agg(u.maker_id))[1] FROM item_units u WHERE u.item_id = ",
            $item,
            "      HAVING bool_and(u.maker_id IS NOT NULL) AND COUNT(DISTINCT u.maker_id) = 1))"
        )
    };
}
pub(crate) use stack_maker_sql;

/// The `items` columns an [`Item`] reads, for `SELECT ... FROM items` (not
/// an alias: the units and maker subqueries name `items.id`). Not for
/// `RETURNING`, which can't see the units an insert's trigger writes.
pub const ITEM_COLUMNS: &str = concat!(
    "id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at, durability, min_quality,      COALESCE((SELECT json_agg(json_build_array(u.quality, u.count) ORDER BY u.quality)                FROM item_units u WHERE u.item_id = items.id), '[]'::json) AS units, ",
    stack_maker_sql!("items.id"),
    " AS maker"
);

/// Static item type definition (from database)
#[derive(Debug, Clone, Default, Serialize, Deserialize, FromRow)]
pub struct ItemType {
    pub id: String,
    pub name: String,
    pub category: String,
    pub stackable: bool,
    pub weight: f32,
    /// Set for a tool (E1.4), with the three below: `rules::tools::ToolType`
    pub tool_type: Option<String>,
    /// The tool's material, `rules::tools::Tier`
    pub tier: Option<String>,
    /// The tool's power at Q50
    pub base_power: Option<f64>,
    /// The tool's uses at Q50
    pub max_durability: Option<i32>,
    /// Strands a unit the economy is balanced to (D2.3)
    pub target_price: Option<i64>,
}

impl ItemType {
    /// What makes this a tool, if it is one.
    pub fn tool(&self) -> Option<CatalogTool> {
        Some(CatalogTool {
            tool_type: self.tool_type.clone()?,
            tier: self.tier.clone()?,
            base_power: self.base_power?,
            max_durability: self.max_durability?,
        })
    }

    /// The durability a new one of these made at `quality` starts with; `None`
    /// unless it's a tool.
    pub fn durability_at(&self, quality: i32) -> Option<i32> {
        self.max_durability.map(|base| tools::max_durability(base, quality))
    }
}

/// Item instance in a player's inventory (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Item {
    pub id: Uuid,
    pub item_type: String,
    pub quality: i32,
    pub quantity: i32,
    pub owner_id: Option<Uuid>,
    pub container_id: Option<Uuid>,
    pub slot_index: Option<i32>,
    pub created_at: DateTime<Utc>,
    /// Uses left, for a tool (E1.4); `None` for anything else.
    pub durability: Option<i32>,
    /// The lowest quality of its units (E1.2, E1.8)
    pub min_quality: i32,
    /// Every unit, a count per quality (E1.8). `quantity`, `quality` and
    /// `min_quality` summarise these.
    #[sqlx(json)]
    pub units: Units,
    /// Who made every unit, if one maker did and their byline is on (#242)
    #[sqlx(default)]
    pub maker: Option<sqlx::types::Json<Maker>>,
}

impl Item {
    /// The client-facing view of this item, of type `item_type` (its name, and its
    /// most durability if it's a tool), or of a type the server no longer knows.
    pub fn to_inventory_item(&self, item_type: Option<&ItemType>) -> InventoryItem {
        InventoryItem {
            id: self.id,
            item_type: self.item_type.clone(),
            name: item_type.map_or_else(|| self.item_type.clone(), |t| t.name.clone()),
            quality: self.quality,
            quality_grade: quality_to_grade(self.quality),
            quantity: self.quantity,
            durability: self.durability,
            max_durability: self.max_durability(item_type),
        }
    }

    /// This item as its slot shows it, or `None` if it isn't in a slot.
    pub fn to_slot_item(&self, item_type: Option<&ItemType>) -> Option<SlotItem> {
        let slot = SlotItem::new(
            self.id,
            self.item_type.clone(),
            item_type.map_or_else(|| self.item_type.clone(), |t| t.name.clone()),
            self.quality,
            self.quantity,
            self.slot_index?,
        )
        .with_units(self.units.clone())
        .with_maker(self.maker.as_ref().map(|m| m.0.clone()));
        Some(match (self.durability, self.max_durability(item_type)) {
            (Some(left), Some(max)) => slot.with_durability(left, max),
            _ => slot,
        })
    }

    /// This tool's most durability, from its type and its own quality. `None` for
    /// anything that isn't a tool.
    pub fn max_durability(&self, item_type: Option<&ItemType>) -> Option<i32> {
        self.durability?;
        item_type?.durability_at(self.quality)
    }
}

/// Convert quality (0-100) to letter grade
pub fn quality_to_grade(quality: i32) -> String {
    capitol_protocol::rules::quality_grade(quality).to_string()
}

use uuid::Uuid;

use crate::models::{ContainerState, MoveResult, SlotItem};

/// Move an item within or between containers
/// Returns the result of the move operation
pub fn move_item(
    source_container: &mut ContainerState,
    target_container: &mut ContainerState,
    item_id: Uuid,
    source_slot: i32,
    target_slot: i32,
) -> Result<MoveResult, String> {
    // Validate slot bounds
    if target_slot < 0 || target_slot >= target_container.slot_count {
        return Err(format!(
            "Target slot {} out of bounds (0-{})",
            target_slot,
            target_container.slot_count - 1
        ));
    }

    // Get source item
    let source_item = source_container
        .get_slot(source_slot)
        .ok_or_else(|| "Source slot is empty".to_string())?
        .clone();

    if source_item.item_id != item_id {
        return Err("Item ID mismatch".to_string());
    }

    // Check target slot
    let target_item = target_container.get_slot(target_slot).cloned();

    let result = match target_item {
        None => {
            // Target empty: simple move
            source_container.slots.remove(&source_slot);
            let mut moved_item = source_item;
            moved_item.slot_index = target_slot;
            target_container.set_slot(target_slot, moved_item);

            MoveResult::Moved {
                item_id,
                to_slot: target_slot,
            }
        }
        Some(target) if target.item_type == source_item.item_type => {
            // Same type: merge stacks
            let merged = merge_items(&source_item, &target);
            source_container.slots.remove(&source_slot);

            let merged_slot_item = SlotItem::new(
                target.item_id,
                target.item_type.clone(),
                target.item_name.clone(),
                merged.quality,
                merged.quantity,
                target_slot,
            );
            target_container.set_slot(target_slot, merged_slot_item);

            MoveResult::Merged {
                source_item_id: source_item.item_id,
                target_item_id: target.item_id,
                new_quantity: merged.quantity,
                new_quality: merged.quality,
            }
        }
        Some(target) => {
            // Different type: swap
            let same_container = source_container.id == target_container.id;

            if same_container {
                // Swap within same container
                let mut moved_source = source_item.clone();
                moved_source.slot_index = target_slot;

                let mut moved_target = target.clone();
                moved_target.slot_index = source_slot;

                source_container.set_slot(source_slot, moved_target.clone());
                source_container.set_slot(target_slot, moved_source);

                MoveResult::Swapped {
                    item1_id: source_item.item_id,
                    item1_to_slot: target_slot,
                    item2_id: target.item_id,
                    item2_to_slot: source_slot,
                }
            } else {
                // Swap between containers
                let mut moved_source = source_item.clone();
                moved_source.slot_index = target_slot;

                let mut moved_target = target.clone();
                moved_target.slot_index = source_slot;

                source_container.set_slot(source_slot, moved_target);
                target_container.set_slot(target_slot, moved_source);

                MoveResult::Swapped {
                    item1_id: source_item.item_id,
                    item1_to_slot: target_slot,
                    item2_id: target.item_id,
                    item2_to_slot: source_slot,
                }
            }
        }
    };

    Ok(result)
}

/// Merged item result
pub struct MergedItem {
    pub quantity: i32,
    pub quality: i32,
}

/// Merge two item stacks of the same type
/// Returns combined quantity and weighted average quality
pub fn merge_items(source: &SlotItem, target: &SlotItem) -> MergedItem {
    let total_qty = source.quantity + target.quantity;
    let weighted_quality =
        (source.quality * source.quantity + target.quality * target.quantity) / total_qty;

    MergedItem {
        quantity: total_qty,
        quality: weighted_quality,
    }
}

/// Find first empty slot in a container, or slot with matching item type for merging
pub fn find_slot_for_item(container: &ContainerState, item_type: &str) -> Option<(i32, bool)> {
    // First, try to find existing stack of same type
    if let Some(slot) = container.find_slot_with_type(item_type) {
        return Some((slot, true)); // true = will merge
    }

    // Otherwise, find empty slot
    if let Some(slot) = container.find_empty_slot() {
        return Some((slot, false)); // false = new slot
    }

    None
}

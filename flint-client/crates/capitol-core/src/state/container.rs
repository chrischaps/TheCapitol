use capitol_protocol::rest::{ContainerResponse, SlotItem, StationContainerResponse};
use uuid::Uuid;

/// A slot container laid out for a grid: one entry per slot, `None` when empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub id: Uuid,
    /// One of [`crate::catalog::container_type`].
    pub container_type: String,
    pub layout_columns: usize,
    pub slots: Vec<Option<SlotItem>>,
}

impl Container {
    pub fn new(
        id: Uuid,
        container_type: &str,
        slot_count: i32,
        layout_columns: i32,
        items: &[SlotItem],
    ) -> Self {
        let mut slots = vec![None; slot_count.max(0) as usize];
        for item in items {
            match usize::try_from(item.slot_index)
                .ok()
                .and_then(|i| slots.get_mut(i))
            {
                Some(slot) => *slot = Some(item.clone()),
                None => tracing::warn!(
                    "container {id}: item {} in slot {} of {slot_count}",
                    item.item_id,
                    item.slot_index
                ),
            }
        }
        Self {
            id,
            container_type: container_type.to_string(),
            layout_columns: layout_columns.max(1) as usize,
            slots,
        }
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn slot(&self, index: usize) -> Option<&SlotItem> {
        self.slots.get(index).and_then(Option::as_ref)
    }

    /// The occupied slots, in slot order.
    pub fn items(&self) -> impl Iterator<Item = &SlotItem> {
        self.slots.iter().flatten()
    }

    pub fn used_slots(&self) -> usize {
        self.items().count()
    }

    /// The slot holding an item, if it's here.
    pub fn find(&self, item_id: Uuid) -> Option<&SlotItem> {
        self.items().find(|item| item.item_id == item_id)
    }

    /// The first empty slot.
    pub fn first_empty(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }
}

impl From<&ContainerResponse> for Container {
    fn from(c: &ContainerResponse) -> Self {
        Self::new(
            c.id,
            &c.container_type,
            c.slot_count,
            c.layout_columns,
            &c.slots,
        )
    }
}

impl From<&StationContainerResponse> for Container {
    fn from(c: &StationContainerResponse) -> Self {
        Self::new(
            c.id,
            &c.container_type,
            c.slot_count,
            c.layout_columns,
            &c.slots,
        )
    }
}

import { useCallback } from 'react'
import { useGame } from '../contexts/GameContext'
import type { DraggedItem } from '../contexts/DragDropContext'
import type { SlotItem } from './InventorySlot'
import InventoryGrid from './InventoryGrid'
import './InventoryPanel.css'

// Map kit item types to station types
const KIT_TO_STATION: Record<string, string> = {
  workbench_kit: 'workbench',
  forge_kit: 'forge',
  chest_kit: 'storage_chest',
}

export default function InventoryPanel() {
  const { inventoryContainer, moveItem, enterPlacementMode, placementMode } = useGame()

  const handleItemDrop = useCallback((targetContainer: string, targetSlot: number, item: DraggedItem) => {
    // Only handle drops if source is different from target
    if (item.sourceContainer === targetContainer && item.sourceSlot === targetSlot) {
      return
    }
    moveItem(item.itemId, targetContainer, targetSlot)
  }, [moveItem])

  const handleItemRightClick = useCallback((item: SlotItem) => {
    // Check if it's a placeable kit item
    const stationType = KIT_TO_STATION[item.itemType]
    if (stationType) {
      enterPlacementMode(stationType, item.itemId)
    }
  }, [enterPlacementMode])

  if (!inventoryContainer) {
    return (
      <div className="inventory-panel-grid">
        <div className="inventory-header">
          <h3>Inventory</h3>
          <span className="inventory-count">Loading...</span>
        </div>
      </div>
    )
  }

  const totalItems = Array.from(inventoryContainer.slots.values())
    .reduce((sum, item) => sum + item.quantity, 0)
  const usedSlots = inventoryContainer.slots.size

  // Convert GameContext SlotItem to component SlotItem
  const convertedSlots = new Map<number, import('./InventorySlot').SlotItem>()
  for (const [idx, item] of inventoryContainer.slots) {
    convertedSlots.set(idx, {
      itemId: item.itemId,
      itemType: item.itemType,
      itemName: item.itemName,
      quality: item.quality,
      qualityGrade: item.qualityGrade,
      quantity: item.quantity,
      slotIndex: item.slotIndex,
    })
  }

  return (
    <div className="inventory-panel-grid">
      <div className="inventory-header">
        <h3>Inventory</h3>
        <span className="inventory-count">{usedSlots}/{inventoryContainer.slotCount}</span>
      </div>
      <InventoryGrid
        containerId={inventoryContainer.id}
        slotCount={inventoryContainer.slotCount}
        columns={inventoryContainer.layoutColumns}
        slots={convertedSlots}
        onItemDrop={handleItemDrop}
        onItemRightClick={handleItemRightClick}
      />
      {placementMode?.active && (
        <div className="placement-mode-hint">
          Click on map to place. Right-click to cancel.
        </div>
      )}
      {totalItems > 0 && (
        <div className="inventory-footer">
          <span className="total-items">{totalItems} items</span>
        </div>
      )}
    </div>
  )
}

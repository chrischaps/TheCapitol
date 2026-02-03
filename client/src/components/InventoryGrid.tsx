import InventorySlot, { type SlotItem } from './InventorySlot'
import type { DraggedItem } from '../contexts/DragDropContext'
import './InventoryGrid.css'

interface InventoryGridProps {
  containerId: string
  slotCount: number
  columns: number
  slots: Map<number, SlotItem>
  disabled?: boolean
  onItemDrop?: (targetContainer: string, targetSlot: number, item: DraggedItem) => void
  onItemRightClick?: (item: SlotItem) => void
}

export default function InventoryGrid({ containerId, slotCount, columns, slots, disabled = false, onItemDrop, onItemRightClick }: InventoryGridProps) {
  const gridStyle = {
    gridTemplateColumns: `repeat(${columns}, 48px)`,
  }

  const slotElements = []
  for (let i = 0; i < slotCount; i++) {
    const item = slots.get(i) || null
    slotElements.push(
      <InventorySlot
        key={i}
        containerId={containerId}
        slotIndex={i}
        item={item}
        disabled={disabled}
        onItemDrop={onItemDrop}
        onItemRightClick={onItemRightClick}
      />
    )
  }

  return (
    <div className="inventory-grid" style={gridStyle}>
      {slotElements}
    </div>
  )
}

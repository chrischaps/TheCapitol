import { useDragDrop, type DraggedItem } from '../contexts/DragDropContext'
import './InventorySlot.css'

export interface SlotItem {
  itemId: string
  itemType: string
  itemName: string
  quality: number
  qualityGrade: string
  quantity: number
  slotIndex: number
}

interface InventorySlotProps {
  containerId: string
  slotIndex: number
  item: SlotItem | null
  disabled?: boolean
  onItemDrop?: (targetContainer: string, targetSlot: number, item: DraggedItem) => void
  onItemRightClick?: (item: SlotItem) => void
}

export default function InventorySlot({ containerId, slotIndex, item, disabled = false, onItemDrop, onItemRightClick }: InventorySlotProps) {
  const { isDragging, draggedItem, startDrag, endDrag } = useDragDrop()

  const isSource = draggedItem?.sourceContainer === containerId && draggedItem?.sourceSlot === slotIndex
  const canDrop = isDragging && !isSource && !disabled

  const handleDragStart = (e: React.DragEvent) => {
    if (!item || disabled) {
      e.preventDefault()
      return
    }

    const dragItem: DraggedItem = {
      itemId: item.itemId,
      itemType: item.itemType,
      itemName: item.itemName,
      quality: item.quality,
      qualityGrade: item.qualityGrade,
      quantity: item.quantity,
      sourceContainer: containerId,
      sourceSlot: slotIndex,
    }
    startDrag(dragItem)

    // Set drag image
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', item.itemId)
  }

  const handleDragEnd = () => {
    endDrag()
  }

  const handleDragOver = (e: React.DragEvent) => {
    if (canDrop) {
      e.preventDefault()
      e.dataTransfer.dropEffect = 'move'
    }
  }

  const handleDropEvent = (e: React.DragEvent) => {
    e.preventDefault()
    if (canDrop && draggedItem && onItemDrop) {
      onItemDrop(containerId, slotIndex, draggedItem)
      endDrag()
    }
  }

  const handleContextMenu = (e: React.MouseEvent) => {
    if (item && onItemRightClick) {
      e.preventDefault()
      onItemRightClick(item)
    }
  }

  const slotClasses = [
    'inventory-slot',
    item ? 'occupied' : 'empty',
    isSource ? 'drag-source' : '',
    canDrop ? 'drop-target' : '',
    disabled ? 'disabled' : '',
  ].filter(Boolean).join(' ')

  const qualityClass = item ? `grade-${item.qualityGrade.toLowerCase()}` : ''

  return (
    <div
      className={slotClasses}
      draggable={!!item && !disabled}
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      onDragOver={handleDragOver}
      onDrop={handleDropEvent}
      onContextMenu={handleContextMenu}
      title={item ? `${item.itemName} (${item.qualityGrade})${item.itemType.endsWith('_kit') ? ' - Right-click to place' : ''}` : 'Empty'}
    >
      {item && (
        <>
          <div className={`slot-quality-pip ${qualityClass}`} />
          <div className="slot-item-icon">
            {/* Simple icon based on item type */}
            {item.itemType === 'fiber' && '\u{1F33F}'}
            {item.itemType === 'rope' && '\u{1FAA2}'}
            {item.itemType === 'workbench_kit' && '\u{1F6E0}'}
            {item.itemType === 'forge_kit' && '\u{1F525}'}
            {item.itemType === 'chest_kit' && '\u{1F4E6}'}
            {!['fiber', 'rope', 'workbench_kit', 'forge_kit', 'chest_kit'].includes(item.itemType) && '\u{1F4E6}'}
          </div>
          <span className="slot-quantity">{item.quantity}</span>
        </>
      )}
    </div>
  )
}

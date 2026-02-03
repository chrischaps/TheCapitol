import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'

export interface DraggedItem {
  itemId: string
  itemType: string
  itemName: string
  quality: number
  qualityGrade: string
  quantity: number
  sourceContainer: string
  sourceSlot: number
}

interface DragDropContextType {
  isDragging: boolean
  draggedItem: DraggedItem | null
  startDrag: (item: DraggedItem) => void
  endDrag: () => void
}

const DragDropContext = createContext<DragDropContextType | null>(null)

export function DragDropProvider({ children }: { children: ReactNode }) {
  const [draggedItem, setDraggedItem] = useState<DraggedItem | null>(null)

  const startDrag = useCallback((item: DraggedItem) => {
    setDraggedItem(item)
  }, [])

  const endDrag = useCallback(() => {
    setDraggedItem(null)
  }, [])

  const value: DragDropContextType = {
    isDragging: draggedItem !== null,
    draggedItem,
    startDrag,
    endDrag,
  }

  return (
    <DragDropContext.Provider value={value}>
      {children}
    </DragDropContext.Provider>
  )
}

export function useDragDrop() {
  const context = useContext(DragDropContext)
  if (!context) {
    throw new Error('useDragDrop must be used within DragDropProvider')
  }
  return context
}

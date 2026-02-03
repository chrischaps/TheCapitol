const API_BASE = '/api'

export interface SlotItem {
  item_id: string
  item_type: string
  item_name: string
  quality: number
  quality_grade: string
  quantity: number
  slot_index: number
}

export interface ContainerState {
  id: string
  container_type: string
  slot_count: number
  layout_columns: number
  slots: SlotItem[]
}

export interface ContainersResponse {
  containers: ContainerState[]
}

export async function getContainers(token: string): Promise<ContainersResponse> {
  const response = await fetch(`${API_BASE}/inventory/containers`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get containers')
  }

  return response.json()
}

export interface MoveItemResponse {
  success: boolean
  action: string
  merged_quantity?: number
  merged_quality?: number
}

export async function moveItem(
  token: string,
  itemId: string,
  targetContainerId: string,
  targetSlot: number
): Promise<MoveItemResponse> {
  const response = await fetch(`${API_BASE}/inventory/move`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({
      item_id: itemId,
      target_container_id: targetContainerId,
      target_slot: targetSlot,
    }),
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to move item')
  }

  return response.json()
}

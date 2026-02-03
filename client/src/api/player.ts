const API_BASE = '/api'

export async function movePlayer(token: string, x: number, y: number): Promise<void> {
  const response = await fetch(`${API_BASE}/player/move`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ x, y }),
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Move failed')
  }
}

export interface InventoryItem {
  id: string
  item_type: string
  name: string
  quality: number
  quality_grade: string
  quantity: number
}

export interface InventoryResponse {
  items: InventoryItem[]
}

export async function getInventory(token: string): Promise<InventoryResponse> {
  const response = await fetch(`${API_BASE}/player/inventory`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get inventory')
  }

  return response.json()
}

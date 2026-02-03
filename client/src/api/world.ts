const API_BASE = '/api'

export interface ResourceNode {
  id: string
  resource_type: string
  x: number
  y: number
  state: string
}

export interface NearbyResponse {
  nodes: ResourceNode[]
}

export async function getNearby(token: string): Promise<NearbyResponse> {
  const response = await fetch(`${API_BASE}/world/nearby`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get nearby nodes')
  }

  return response.json()
}

export async function startExtraction(token: string, nodeId: string): Promise<void> {
  const response = await fetch(`${API_BASE}/world/extract`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ node_id: nodeId }),
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to start extraction')
  }
}

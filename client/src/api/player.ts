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

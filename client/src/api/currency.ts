const API_BASE = '/api'

export interface CurrencyResponse {
  strand_balance: number
}

export async function getCurrency(token: string): Promise<CurrencyResponse> {
  const res = await fetch(`${API_BASE}/player/currency`, {
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })
  if (!res.ok) {
    throw new Error(`Failed to fetch currency: ${res.status}`)
  }
  return res.json()
}

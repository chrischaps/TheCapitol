const API_BASE = '/api'

export interface DepositRequest {
  item_ids: string[]
}

export interface DepositResponse {
  deposited_quantity: number
  fee: number
  strands_received: number
  new_balance: number
}

export interface WithdrawRequest {
  quantity: number
}

export interface WithdrawResponse {
  strands_spent: number
  fee: number
  fiber_received: number
  new_balance: number
}

export async function depositFiber(token: string, itemIds: string[]): Promise<DepositResponse> {
  const res = await fetch(`${API_BASE}/exchange/deposit`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ item_ids: itemIds }),
  })
  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: 'Unknown error' }))
    throw new Error(error.error || `Failed to deposit: ${res.status}`)
  }
  return res.json()
}

export async function withdrawFiber(token: string, quantity: number): Promise<WithdrawResponse> {
  const res = await fetch(`${API_BASE}/exchange/withdraw`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ quantity }),
  })
  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: 'Unknown error' }))
    throw new Error(error.error || `Failed to withdraw: ${res.status}`)
  }
  return res.json()
}

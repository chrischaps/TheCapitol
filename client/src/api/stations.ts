const API_BASE = '/api'

export interface StationInfo {
  id: string
  station_type: string
  name: string
  category: string
  icon?: string
  x: number
  y: number
  owner_id: string
  container_id?: string
  interaction_range: number
}

export interface NearbyStationsResponse {
  stations: StationInfo[]
}

export async function getNearbyStations(token: string): Promise<NearbyStationsResponse> {
  const response = await fetch(`${API_BASE}/stations/nearby`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get nearby stations')
  }

  return response.json()
}

export interface PlaceStationRequest {
  station_type: string
  x: number
  y: number
  kit_item_id: string
}

export interface PlaceStationResponse {
  station: StationInfo
}

export async function placeStation(
  token: string,
  stationType: string,
  x: number,
  y: number,
  kitItemId: string
): Promise<PlaceStationResponse> {
  const response = await fetch(`${API_BASE}/stations/place`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({
      station_type: stationType,
      x,
      y,
      kit_item_id: kitItemId,
    }),
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to place station')
  }

  return response.json()
}

export async function removeStation(token: string, stationId: string): Promise<void> {
  const response = await fetch(`${API_BASE}/stations/${stationId}`, {
    method: 'DELETE',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to remove station')
  }
}

export interface StationContainerResponse {
  id: string
  container_type: string
  slot_count: number
  layout_columns: number
  slots: Array<{
    item_id: string
    item_type: string
    item_name: string
    quality: number
    quality_grade: string
    quantity: number
    slot_index: number
  }>
}

export async function getStationContainer(
  token: string,
  stationId: string
): Promise<StationContainerResponse> {
  const response = await fetch(`${API_BASE}/stations/${stationId}/container`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get station container')
  }

  return response.json()
}

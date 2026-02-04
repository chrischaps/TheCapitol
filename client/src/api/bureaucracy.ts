const API_BASE = 'http://localhost:3000'

export interface ZoneInfo {
  id: string
  name: string
  ringOrder: number
  innerRadius: number
  outerRadius: number
  hasPlots: boolean
  plotSizeCategory: string | null
  deedCost: number | null
  description: string | null
}

export interface DeedPrice {
  zoneId: string
  zoneName: string
  deedCost: number
  plotSize: string
  stationCapacity: number
}

export interface ZonesResponse {
  zones: ZoneInfo[]
}

export interface DeedPricesResponse {
  prices: DeedPrice[]
}

export async function getZones(token: string): Promise<ZonesResponse> {
  const res = await fetch(`${API_BASE}/bureaucracy/zones`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get zones: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    zones: data.zones.map((z: {
      id: string
      name: string
      ring_order: number
      inner_radius: number
      outer_radius: number
      has_plots: boolean
      plot_size_category: string | null
      deed_cost: number | null
      description: string | null
    }) => ({
      id: z.id,
      name: z.name,
      ringOrder: z.ring_order,
      innerRadius: z.inner_radius,
      outerRadius: z.outer_radius,
      hasPlots: z.has_plots,
      plotSizeCategory: z.plot_size_category,
      deedCost: z.deed_cost,
      description: z.description,
    })),
  }
}

export async function getDeedPrices(token: string): Promise<DeedPricesResponse> {
  const res = await fetch(`${API_BASE}/bureaucracy/deed-prices`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get deed prices: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    prices: data.prices.map((p: {
      zone_id: string
      zone_name: string
      deed_cost: number
      plot_size: string
      station_capacity: number
    }) => ({
      zoneId: p.zone_id,
      zoneName: p.zone_name,
      deedCost: p.deed_cost,
      plotSize: p.plot_size,
      stationCapacity: p.station_capacity,
    })),
  }
}

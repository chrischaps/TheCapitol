const API_BASE = 'http://localhost:3000'

export interface Bounds {
  minX: number
  minY: number
  maxX: number
  maxY: number
}

export interface PlotInfo {
  id: string
  zoneId: string
  worldX: number
  worldY: number
  bounds: Bounds
  sizeCategory: string
  plotType: string
  ownerId: string | null
  ownerName: string | null
  claimedAt: string | null
  assessedValue: number
  stationCount: number
  stationCapacity: number
  /** The `/terrain` lot this plot is. Null only for a pre-B7 plot. */
  terrainId: string | null
  /** The lot's corners: the shape that's claimed and that placement tests. */
  corners: [number, number][] | null
}

/** Whether a point is on a plot: inside its lot, or its box if it has none. */
export function plotContains(plot: PlotInfo, x: number, y: number): boolean {
  const c = plot.corners
  if (!c) {
    const b = plot.bounds
    return x >= b.minX && x <= b.maxX && y >= b.minY && y <= b.maxY
  }
  // Even-odd crossing test. The server's test also counts the edge, which
  // matters only to a click exactly on the line.
  let inside = false
  for (let i = 0, j = c.length - 1; i < c.length; j = i++) {
    const [xi, yi] = c[i]
    const [xj, yj] = c[j]
    if ((yi > y) !== (yj > y) && x < xi + ((y - yi) * (xj - xi)) / (yj - yi)) {
      inside = !inside
    }
  }
  return inside
}

export interface ListPlotsResponse {
  plots: PlotInfo[]
  total: number
}

export interface ClaimPlotResponse {
  plot: PlotInfo
  newBalance: number
}

export interface PlotPermissions {
  plotId: string
  entryLevel: string
  visibilityLevel: string
  stationUseLevel: string
  modifyLevel: string
}

interface ApiPlotInfo {
  id: string
  zone_id: string
  world_x: number
  world_y: number
  bounds: {
    min_x: number
    min_y: number
    max_x: number
    max_y: number
  }
  size_category: string
  plot_type: string
  owner_id: string | null
  owner_name: string | null
  claimed_at: string | null
  assessed_value: number
  station_count: number
  station_capacity: number
  terrain_id?: string | null
  corners?: [number, number][] | null
}

function transformPlot(api: ApiPlotInfo): PlotInfo {
  return {
    id: api.id,
    zoneId: api.zone_id,
    worldX: api.world_x,
    worldY: api.world_y,
    bounds: {
      minX: api.bounds.min_x,
      minY: api.bounds.min_y,
      maxX: api.bounds.max_x,
      maxY: api.bounds.max_y,
    },
    sizeCategory: api.size_category,
    plotType: api.plot_type,
    ownerId: api.owner_id,
    ownerName: api.owner_name,
    claimedAt: api.claimed_at,
    assessedValue: api.assessed_value,
    stationCount: api.station_count,
    stationCapacity: api.station_capacity,
    terrainId: api.terrain_id ?? null,
    corners: api.corners ?? null,
  }
}

export async function getPlots(
  token: string,
  options?: {
    zone?: string
    unclaimed?: boolean
    owner?: string
    limit?: number
    offset?: number
  }
): Promise<ListPlotsResponse> {
  const params = new URLSearchParams()
  if (options?.zone) params.set('zone', options.zone)
  if (options?.unclaimed) params.set('unclaimed', 'true')
  if (options?.owner) params.set('owner', options.owner)
  if (options?.limit) params.set('limit', String(options.limit))
  if (options?.offset) params.set('offset', String(options.offset))

  const url = `${API_BASE}/plots?${params.toString()}`
  const res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get plots: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    plots: data.plots.map(transformPlot),
    total: data.total,
  }
}

export async function getPlot(token: string, plotId: string): Promise<PlotInfo> {
  const res = await fetch(`${API_BASE}/plots/${plotId}`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get plot: ${res.statusText}`)
  }

  const data = await res.json()
  return transformPlot(data)
}

export async function getPlotAtPosition(
  token: string,
  x: number,
  y: number
): Promise<PlotInfo | null> {
  const res = await fetch(`${API_BASE}/plots/at?x=${x}&y=${y}`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get plot at position: ${res.statusText}`)
  }

  const data = await res.json()
  return data ? transformPlot(data) : null
}

export async function claimPlot(
  token: string,
  plotId: string
): Promise<ClaimPlotResponse> {
  const res = await fetch(`${API_BASE}/plots/${plotId}/claim`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
  })

  if (!res.ok) {
    const error = await res.json().catch(() => ({ message: res.statusText }))
    throw new Error(error.message || `Failed to claim plot: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    plot: transformPlot(data.plot),
    newBalance: data.new_balance,
  }
}

export async function getPlotPermissions(
  token: string,
  plotId: string
): Promise<PlotPermissions> {
  const res = await fetch(`${API_BASE}/plots/${plotId}/permissions`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  if (!res.ok) {
    throw new Error(`Failed to get permissions: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    plotId: data.plot_id,
    entryLevel: data.entry_level,
    visibilityLevel: data.visibility_level,
    stationUseLevel: data.station_use_level,
    modifyLevel: data.modify_level,
  }
}

export async function updatePlotPermissions(
  token: string,
  plotId: string,
  permissions: Partial<{
    entryLevel: string
    visibilityLevel: string
    stationUseLevel: string
    modifyLevel: string
  }>
): Promise<PlotPermissions> {
  const res = await fetch(`${API_BASE}/plots/${plotId}/permissions`, {
    method: 'PUT',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      entry_level: permissions.entryLevel,
      visibility_level: permissions.visibilityLevel,
      station_use_level: permissions.stationUseLevel,
      modify_level: permissions.modifyLevel,
    }),
  })

  if (!res.ok) {
    throw new Error(`Failed to update permissions: ${res.statusText}`)
  }

  const data = await res.json()
  return {
    plotId: data.plot_id,
    entryLevel: data.entry_level,
    visibilityLevel: data.visibility_level,
    stationUseLevel: data.station_use_level,
    modifyLevel: data.modify_level,
  }
}

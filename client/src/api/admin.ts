const API_URL = import.meta.env.VITE_API_URL || 'http://localhost:3000'

export interface SuburbanGenParams {
  inner_radius: number
  outer_radius: number
  collector_width: number
  local_street_width: number
  min_street_spacing: number
  min_plot_area: number
  max_plot_area: number
  target_plot_area: number
  min_frontage: number
  max_frontage: number
  seed: number
  // Suburb v2 (#103): which generator lays out the suburb
  layout: SuburbLayout
  // The Neighbourhood generator's knobs. Left out, the server fills its defaults.
  neighbourhood?: NeighbourhoodParams
}

export type SuburbLayout = 'legacy' | 'neighbourhood'

export type NeighbourhoodPattern = 'warped_parallels' | 'loops_and_lollipops' | 'central_loop'

export interface NeighbourhoodParams {
  lot_frontage: number
  lot_depth: number
  lot_depth_min: number
  lot_depth_max: number
  local_width: number
  collector_width: number
  buffer: number
  split_chance: number
  greenway_width: number
  link_spacing: number
  bulb_radius: number
  cul_de_sac_max: number
  warp_amplitude: number
  warp_wavelength: number
  pattern_weights: Record<NeighbourhoodPattern, number>
  force_pattern: NeighbourhoodPattern | null
}

export interface WorldGenParams {
  // Zone radii
  capitol_radius: number
  trade_outer_radius: number
  guild_outer_radius: number
  urban_outer_radius: number
  suburban_outer_radius: number
  rural_outer_radius: number
  // Water/Canals
  moat_width: number
  moat_bridge_width: number
  canal_inner_width: number
  canal_outer_width: number
  canal_bridge_width: number
  // Ring roads
  guild_ring_radius: number
  guild_ring_width: number
  trade_ring_radius: number
  trade_ring_width: number
  // Arterials
  arterial_width: number
  arterial_end_radius: number
  // Urban grid
  urban_radial_spacing: number
  urban_spoke_count: number
  urban_street_width: number
  // Urban plots
  urban_plot_frontage: number
  urban_plot_depth: number
  // Suburban
  suburban: SuburbanGenParams
  // Rural
  rural_trail_width: number
  rural_trail_count: number
  // Rural plots
  rural_plot_frontage: number
  rural_plot_depth: number
  // Guild halls
  guild_plot_frontage: number
  // Seeds
  seed: number
}

export interface ServerStats {
  tps: number
  tick_count: number
  uptime_seconds: number
  active_players: number
  active_crafting: number
  active_extracting: number
  active_trades: number
  resource_nodes: number
  station_count: number
  command_queue_size: number
  broadcast_subscribers: number
}

export async function getServerStats(token: string): Promise<ServerStats> {
  const response = await fetch(`${API_URL}/admin/stats`, {
    headers: { Authorization: `Bearer ${token}` },
  })
  if (!response.ok) {
    throw new Error(`Failed to fetch server stats: ${response.status}`)
  }
  return response.json()
}

export async function getWorldParams(token: string): Promise<WorldGenParams> {
  const response = await fetch(`${API_URL}/admin/world-params`, {
    headers: { Authorization: `Bearer ${token}` },
  })
  if (!response.ok) {
    throw new Error(`Failed to fetch world params: ${response.status}`)
  }
  return response.json()
}

// The server's defaults, for the form's reset. The client holds no copy, so the
// zone radii live only in capitol-protocol's rules.rs (#116).
export async function getDefaultWorldParams(token: string): Promise<WorldGenParams> {
  const response = await fetch(`${API_URL}/admin/world-params/default`, {
    headers: { Authorization: `Bearer ${token}` },
  })
  if (!response.ok) {
    throw new Error(`Failed to fetch default world params: ${response.status}`)
  }
  return response.json()
}

export async function updateWorldParams(token: string, params: WorldGenParams): Promise<WorldGenParams> {
  const response = await fetch(`${API_URL}/admin/world-params`, {
    method: 'PUT',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(params),
  })
  if (!response.ok) {
    throw new Error(`Failed to update world params: ${response.status}`)
  }
  return response.json()
}

export async function commitWorldParams(token: string): Promise<WorldGenParams> {
  const response = await fetch(`${API_URL}/admin/world-params/commit`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}` },
  })
  if (!response.ok) {
    throw new Error(`Failed to commit world params: ${response.status}`)
  }
  return response.json()
}

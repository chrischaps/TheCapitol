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

export const DEFAULT_WORLD_PARAMS: WorldGenParams = {
  capitol_radius: 200,
  trade_outer_radius: 600,
  guild_outer_radius: 1000,
  urban_outer_radius: 1600,
  suburban_outer_radius: 2400,
  rural_outer_radius: 3400,
  moat_width: 20,
  moat_bridge_width: 30,
  canal_inner_width: 15,
  canal_outer_width: 15,
  canal_bridge_width: 25,
  guild_ring_radius: 800,
  guild_ring_width: 30,
  trade_ring_radius: 400,
  trade_ring_width: 30,
  arterial_width: 60,
  arterial_end_radius: 3800,
  urban_radial_spacing: 100,
  urban_spoke_count: 48,
  urban_street_width: 16,
  urban_plot_frontage: 40,
  urban_plot_depth: 50,
  suburban: {
    inner_radius: 1600,
    outer_radius: 2400,
    collector_width: 14,
    local_street_width: 10,
    min_street_spacing: 100,
    min_plot_area: 6000,
    max_plot_area: 12000,
    target_plot_area: 8000,
    min_frontage: 60,
    max_frontage: 100,
    seed: 42,
  },
  rural_trail_width: 6,
  rural_trail_count: 12,
  rural_plot_frontage: 200,
  rural_plot_depth: 150,
  guild_plot_frontage: 150,
  seed: 42,
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

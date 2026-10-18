const API_URL = import.meta.env.VITE_API_URL || 'http://localhost:3000'

export interface Bridge {
  angle_degrees: number
  width: number
}

export interface WaterRing {
  id: string
  radius: number
  width: number
  bridges: Bridge[]
}

export interface RoadSegment {
  id: string
  road_type: 'arterial' | 'grid' | 'suburban' | 'trail'
  points: [number, number][]
  width: number
}

export interface TerrainPlot {
  id: string
  corners: [[number, number], [number, number], [number, number], [number, number]]
  street_facing_edge: [number, number]
  area: number
  adjacent_street_id: string
  zone: string
}

/** A suburban park: open ground no lot took. Its rings fill even-odd, so a ring inside another is a hole. */
export interface TerrainPark {
  id: string
  rings: [number, number][][]
  area: number
}

export interface TerrainData {
  world_size: number
  center_x: number
  center_y: number
  water_rings: WaterRing[]
  roads: RoadSegment[]
  terrain_plots: TerrainPlot[]
  /** Absent from servers older than #129 */
  parks?: TerrainPark[]
}

export async function getTerrain(): Promise<TerrainData> {
  const response = await fetch(`${API_URL}/terrain`)
  if (!response.ok) {
    throw new Error('Failed to fetch terrain')
  }
  return response.json()
}

import { useGame } from '../contexts/GameContext'
import './ZoneIndicator.css'

const ZONE_COLORS: Record<string, string> = {
  capitol: '#c9a227',
  trade_district: '#3b82f6',
  guild_district: '#8b5cf6',
  urban: '#64748b',
  suburban: '#22c55e',
  rural: '#84cc16',
  wilderness: '#6b7280',
}

export default function ZoneIndicator() {
  const { currentZone } = useGame()

  if (!currentZone) {
    return null
  }

  const color = ZONE_COLORS[currentZone.id] || '#6b7280'

  return (
    <div className="zone-indicator" style={{ borderColor: color }}>
      <span className="zone-icon" style={{ backgroundColor: color }} />
      <span className="zone-name">{currentZone.name}</span>
    </div>
  )
}

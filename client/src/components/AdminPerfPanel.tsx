import { useState, useEffect, useRef, useCallback } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { useGame } from '../contexts/GameContext'
import { getServerStats, type ServerStats } from '../api/admin'
import './AdminPerfPanel.css'

interface StatRowProps {
  label: string
  value: string | number
  color?: 'good' | 'warn' | 'bad' | 'neutral'
}

function StatRow({ label, value, color = 'neutral' }: StatRowProps) {
  return (
    <div className="stat-row">
      <span className="stat-row-label">{label}</span>
      <span className={`stat-row-value stat-${color}`}>{value}</span>
    </div>
  )
}

function tpsColor(tps: number): 'good' | 'warn' | 'bad' {
  if (tps >= 9.5) return 'good'
  if (tps >= 8) return 'warn'
  return 'bad'
}

function fpsColor(fps: number): 'good' | 'warn' | 'bad' {
  if (fps >= 55) return 'good'
  if (fps >= 30) return 'warn'
  return 'bad'
}

function formatUptime(seconds: number): string {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  return `${h}h ${m}m ${s}s`
}

interface SectionProps {
  title: string
  children: React.ReactNode
  defaultOpen?: boolean
}

function Section({ title, children, defaultOpen = false }: SectionProps) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="admin-perf-section">
      <div className="admin-perf-section-header" onClick={() => setOpen(!open)}>
        <h4>{title}</h4>
        <span className={`chevron ${open ? 'open' : ''}`}>&#9654;</span>
      </div>
      {open && <div className="admin-perf-section-body">{children}</div>}
    </div>
  )
}

export default function AdminPerfPanel() {
  const { token, isAdmin } = useAuth()
  const { players, nodes, stations, isConnected, wsMessagesPerSecond, terrain } = useGame()
  const [isOpen, setIsOpen] = useState(false)
  const [stats, setStats] = useState<ServerStats | null>(null)
  const [error, setError] = useState<string | null>(null)

  // FPS tracking
  const [fps, setFps] = useState(0)
  const [frameTime, setFrameTime] = useState(0)
  const frameSamplesRef = useRef<number[]>([])
  const rafIdRef = useRef<number>(0)
  const lastFrameRef = useRef<number>(0)

  // F3 key toggle (admin only)
  useEffect(() => {
    if (!isAdmin) return

    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'F3') {
        e.preventDefault()
        setIsOpen((prev) => !prev)
      }
    }
    window.addEventListener('keydown', handleKey)
    return () => window.removeEventListener('keydown', handleKey)
  }, [isAdmin])

  // FPS measurement via rAF loop
  useEffect(() => {
    if (!isOpen) return

    const measure = (now: number) => {
      const delta = now - lastFrameRef.current
      lastFrameRef.current = now

      const samples = frameSamplesRef.current
      samples.push(delta)
      if (samples.length > 60) samples.shift()

      const avgDelta = samples.reduce((a, b) => a + b, 0) / samples.length
      setFps(Math.round(1000 / avgDelta))
      setFrameTime(Number(avgDelta.toFixed(1)))

      rafIdRef.current = requestAnimationFrame(measure)
    }

    lastFrameRef.current = performance.now()
    frameSamplesRef.current = []
    rafIdRef.current = requestAnimationFrame(measure)

    return () => cancelAnimationFrame(rafIdRef.current)
  }, [isOpen])

  // Poll server stats every 2s when open
  const fetchStats = useCallback(async () => {
    if (!token) return
    try {
      const data = await getServerStats(token)
      setStats(data)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch stats')
    }
  }, [token])

  useEffect(() => {
    if (!isOpen || !token) return

    fetchStats()
    const interval = setInterval(fetchStats, 2000)
    return () => clearInterval(interval)
  }, [isOpen, token, fetchStats])

  if (!isAdmin || !isOpen) return null

  const canvasEl = document.querySelector('canvas')
  const canvasRes = canvasEl
    ? `${canvasEl.width}x${canvasEl.height}`
    : 'N/A'

  return (
    <div className="admin-perf-panel">
      <div className="admin-perf-header">
        <h3>Performance</h3>
        <button className="close-button" onClick={() => setIsOpen(false)}>X</button>
      </div>

      <div className="admin-perf-content">
        <Section title="Server" defaultOpen>
          {error && <div className="admin-perf-error">{error}</div>}
          {stats ? (
            <>
              <StatRow label="TPS" value={stats.tps.toFixed(1)} color={tpsColor(stats.tps)} />
              <StatRow label="Tick" value={stats.tick_count.toLocaleString()} />
              <StatRow label="Players" value={stats.active_players} />
              <StatRow label="Crafting" value={stats.active_crafting} />
              <StatRow label="Extracting" value={stats.active_extracting} />
              <StatRow label="Trades" value={stats.active_trades} />
              <StatRow label="Resources" value={stats.resource_nodes} />
              <StatRow label="Stations" value={stats.station_count} />
              <StatRow label="Cmd Queue" value={stats.command_queue_size} />
              <StatRow label="WS Subs" value={stats.broadcast_subscribers} />
            </>
          ) : (
            !error && <StatRow label="Loading..." value="" />
          )}
        </Section>

        <Section title="Client" defaultOpen>
          <StatRow label="FPS" value={fps} color={fpsColor(fps)} />
          <StatRow label="Frame" value={`${frameTime}ms`} color={frameTime > 33 ? 'bad' : frameTime > 18 ? 'warn' : 'good'} />
          <StatRow label="WebSocket" value={isConnected ? 'Connected' : 'Disconnected'} color={isConnected ? 'good' : 'bad'} />
          <StatRow label="WS msg/s" value={wsMessagesPerSecond} />
          <StatRow label="Players" value={players.size} />
          <StatRow label="Resources" value={nodes.size} />
          <StatRow label="Stations" value={stations.size} />
          <StatRow label="Roads" value={terrain?.roads?.length ?? 0} />
          <StatRow label="Canvas" value={canvasRes} />
        </Section>
      </div>

      {stats && (
        <div className="admin-perf-uptime">
          Uptime: {formatUptime(stats.uptime_seconds)}
        </div>
      )}
    </div>
  )
}

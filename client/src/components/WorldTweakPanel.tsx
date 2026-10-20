import { useState, useEffect, useCallback } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { getWorldParams, getDefaultWorldParams, updateWorldParams, commitWorldParams, type NeighbourhoodPattern, type WorldGenParams } from '../api/admin'
import './WorldTweakPanel.css'

interface ParamSliderProps {
  label: string
  value: number
  min: number
  max: number
  step: number
  onChange: (value: number) => void
}

function ParamSlider({ label, value, min, max, step, onChange }: ParamSliderProps) {
  return (
    <div className="param-slider">
      <div className="param-slider-header">
        <span className="param-slider-label">{label}</span>
        <input
          className="param-slider-value"
          type="number"
          value={value}
          step={step}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  )
}

interface ParamSelectProps<T extends string> {
  label: string
  value: T
  options: { value: T; label: string }[]
  onChange: (value: T) => void
}

function ParamSelect<T extends string>({ label, value, options, onChange }: ParamSelectProps<T>) {
  return (
    <div className="param-slider">
      <div className="param-slider-header">
        <span className="param-slider-label">{label}</span>
        <select className="param-select" value={value} onChange={(e) => onChange(e.target.value as T)}>
          {options.map((o) => (
            <option key={o.value} value={o.value}>{o.label}</option>
          ))}
        </select>
      </div>
    </div>
  )
}

// '' is no forced pattern: each neighbourhood picks one by the weights.
const SUBURB_PATTERNS: { value: NeighbourhoodPattern | ''; label: string }[] = [
  { value: '', label: 'Mixed' },
  { value: 'warped_parallels', label: 'Warped parallels' },
  { value: 'loops_and_lollipops', label: 'Loops and lollipops' },
  { value: 'central_loop', label: 'Central loop' },
]

interface SectionProps {
  title: string
  children: React.ReactNode
  defaultOpen?: boolean
}

function Section({ title, children, defaultOpen = false }: SectionProps) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="world-tweak-section">
      <div className="world-tweak-section-header" onClick={() => setOpen(!open)}>
        <h4>{title}</h4>
        <span className={`chevron ${open ? 'open' : ''}`}>&#9654;</span>
      </div>
      {open && <div className="world-tweak-section-body">{children}</div>}
    </div>
  )
}

export default function WorldTweakPanel() {
  const { token, isAdmin } = useAuth()
  const [isOpen, setIsOpen] = useState(false)
  const [params, setParams] = useState<WorldGenParams | null>(null)
  const [loading, setLoading] = useState(false)
  const [regenerating, setRegenerating] = useState(false)
  const [committing, setCommitting] = useState(false)
  const [committed, setCommitted] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // F2 key toggle (admin only)
  useEffect(() => {
    if (!isAdmin) return

    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'F2') {
        e.preventDefault()
        setIsOpen((prev) => !prev)
      }
    }
    window.addEventListener('keydown', handleKey)
    return () => window.removeEventListener('keydown', handleKey)
  }, [isAdmin])

  // Fetch params when panel opens
  useEffect(() => {
    if (!isOpen || !token || params) return

    setLoading(true)
    setError(null)
    getWorldParams(token)
      .then((data) => setParams(data))
      .catch((err) => setError(err.message))
      .finally(() => setLoading(false))
  }, [isOpen, token, params])

  const updateParam = useCallback((path: string, value: number | string | null) => {
    setParams((prev) => {
      if (!prev) return prev
      const next = structuredClone(prev)
      const parts = path.split('.')
      let obj: Record<string, unknown> = next as unknown as Record<string, unknown>
      for (let i = 0; i < parts.length - 1; i++) {
        obj = obj[parts[i]] as Record<string, unknown>
      }
      obj[parts[parts.length - 1]] = value
      return next as unknown as WorldGenParams
    })
  }, [])

  const handleRegenerate = useCallback(async () => {
    if (!token || !params) return
    setRegenerating(true)
    setError(null)
    setCommitted(false)
    try {
      const result = await updateWorldParams(token, params)
      setParams(result)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to regenerate')
    } finally {
      setRegenerating(false)
    }
  }, [token, params])

  const handleReset = useCallback(async () => {
    if (!token) return
    setError(null)
    try {
      setParams(await getDefaultWorldParams(token))
      setCommitted(false)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch defaults')
    }
  }, [token])

  const handleCommit = useCallback(async () => {
    if (!token) return
    setCommitting(true)
    setError(null)
    try {
      await commitWorldParams(token)
      setCommitted(true)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to commit')
    } finally {
      setCommitting(false)
    }
  }, [token])

  if (!isAdmin || !isOpen) return null

  return (
    <div className="world-tweak-panel">
      <div className="world-tweak-header">
        <h3>World Tweaker</h3>
        <button className="close-button" onClick={() => setIsOpen(false)}>X</button>
      </div>

      {loading && <div className="world-tweak-loading">Loading parameters...</div>}
      {error && <div className="world-tweak-error">{error}</div>}

      {params && !loading && (
        <>
          <div className="world-tweak-content">
            <Section title="Water & Canals" defaultOpen>
              <ParamSlider label="Moat Width" value={params.moat_width} min={5} max={80} step={1} onChange={(v) => updateParam('moat_width', v)} />
              <ParamSlider label="Moat Bridge Width" value={params.moat_bridge_width} min={10} max={100} step={1} onChange={(v) => updateParam('moat_bridge_width', v)} />
              <ParamSlider label="Inner Canal Width" value={params.canal_inner_width} min={5} max={60} step={1} onChange={(v) => updateParam('canal_inner_width', v)} />
              <ParamSlider label="Outer Canal Width" value={params.canal_outer_width} min={5} max={60} step={1} onChange={(v) => updateParam('canal_outer_width', v)} />
              <ParamSlider label="Canal Bridge Width" value={params.canal_bridge_width} min={10} max={80} step={1} onChange={(v) => updateParam('canal_bridge_width', v)} />
            </Section>

            <Section title="Ring Roads">
              <ParamSlider label="Guild Ring Radius" value={params.guild_ring_radius} min={300} max={1200} step={10} onChange={(v) => updateParam('guild_ring_radius', v)} />
              <ParamSlider label="Guild Ring Width" value={params.guild_ring_width} min={10} max={80} step={1} onChange={(v) => updateParam('guild_ring_width', v)} />
              <ParamSlider label="Trade Ring Radius" value={params.trade_ring_radius} min={100} max={800} step={10} onChange={(v) => updateParam('trade_ring_radius', v)} />
              <ParamSlider label="Trade Ring Width" value={params.trade_ring_width} min={10} max={80} step={1} onChange={(v) => updateParam('trade_ring_width', v)} />
            </Section>

            <Section title="Arterials">
              <ParamSlider label="Arterial Width" value={params.arterial_width} min={10} max={150} step={1} onChange={(v) => updateParam('arterial_width', v)} />
              <ParamSlider label="Arterial End Radius" value={params.arterial_end_radius} min={2000} max={6000} step={50} onChange={(v) => updateParam('arterial_end_radius', v)} />
            </Section>

            <Section title="Guild Halls">
              <ParamSlider label="Plot Frontage" value={params.guild_plot_frontage} min={60} max={400} step={10} onChange={(v) => updateParam('guild_plot_frontage', v)} />
            </Section>

            <Section title="Urban Grid">
              <ParamSlider label="Radial Spacing" value={params.urban_radial_spacing} min={30} max={300} step={5} onChange={(v) => updateParam('urban_radial_spacing', v)} />
              <ParamSlider label="Spoke Count" value={params.urban_spoke_count} min={8} max={128} step={1} onChange={(v) => updateParam('urban_spoke_count', v)} />
              <ParamSlider label="Street Width" value={params.urban_street_width} min={4} max={40} step={1} onChange={(v) => updateParam('urban_street_width', v)} />
              <ParamSlider label="Plot Frontage" value={params.urban_plot_frontage} min={20} max={100} step={1} onChange={(v) => updateParam('urban_plot_frontage', v)} />
              <ParamSlider label="Plot Depth" value={params.urban_plot_depth} min={20} max={100} step={1} onChange={(v) => updateParam('urban_plot_depth', v)} />
            </Section>

            <Section title="Suburban">
              <ParamSelect label="Pattern" value={params.suburban.neighbourhood.force_pattern ?? ''} options={SUBURB_PATTERNS} onChange={(v) => updateParam('suburban.neighbourhood.force_pattern', v || null)} />
              <ParamSlider label="Collector Width" value={params.suburban.neighbourhood.collector_width} min={10} max={40} step={1} onChange={(v) => updateParam('suburban.neighbourhood.collector_width', v)} />
              <ParamSlider label="Local Street Width" value={params.suburban.neighbourhood.local_width} min={8} max={30} step={1} onChange={(v) => updateParam('suburban.neighbourhood.local_width', v)} />
              <ParamSlider label="Lot Frontage" value={params.suburban.neighbourhood.lot_frontage} min={40} max={100} step={1} onChange={(v) => updateParam('suburban.neighbourhood.lot_frontage', v)} />
              <ParamSlider label="Lot Depth" value={params.suburban.neighbourhood.lot_depth} min={params.suburban.neighbourhood.lot_depth_min} max={params.suburban.neighbourhood.lot_depth_max} step={1} onChange={(v) => updateParam('suburban.neighbourhood.lot_depth', v)} />
              <ParamSlider label="Split Chance" value={params.suburban.neighbourhood.split_chance} min={0} max={1} step={0.05} onChange={(v) => updateParam('suburban.neighbourhood.split_chance', v)} />
              <ParamSlider label="Warp Amplitude" value={params.suburban.neighbourhood.warp_amplitude} min={0} max={40} step={1} onChange={(v) => updateParam('suburban.neighbourhood.warp_amplitude', v)} />
            </Section>

            <Section title="Rural">
              <ParamSlider label="Trail Width" value={params.rural_trail_width} min={2} max={20} step={1} onChange={(v) => updateParam('rural_trail_width', v)} />
              <ParamSlider label="Trail Count" value={params.rural_trail_count} min={2} max={36} step={1} onChange={(v) => updateParam('rural_trail_count', v)} />
              <ParamSlider label="Plot Frontage" value={params.rural_plot_frontage} min={100} max={400} step={10} onChange={(v) => updateParam('rural_plot_frontage', v)} />
              <ParamSlider label="Plot Depth" value={params.rural_plot_depth} min={80} max={300} step={10} onChange={(v) => updateParam('rural_plot_depth', v)} />
            </Section>

            <Section title="Seed">
              <ParamSlider label="World Seed" value={params.seed} min={0} max={9999} step={1} onChange={(v) => updateParam('seed', v)} />
            </Section>
          </div>

          <div className="world-tweak-actions">
            <button className="world-tweak-regenerate" onClick={handleRegenerate} disabled={regenerating}>
              {regenerating ? 'Regenerating...' : 'Regenerate'}
            </button>
            <button className="world-tweak-commit" onClick={handleCommit} disabled={committing || committed}>
              {committing ? 'Saving...' : committed ? 'Committed' : 'Commit'}
            </button>
            <button className="world-tweak-reset" onClick={handleReset}>
              Reset
            </button>
          </div>
        </>
      )}
    </div>
  )
}

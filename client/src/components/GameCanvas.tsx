import { useRef, useEffect, useCallback, useState } from 'react'
import { useGame } from '../contexts/GameContext'
import { useAuth } from '../contexts/AuthContext'
import type { ResourceNode, PlayerPosition } from '../contexts/GameContext'
import type { StationInfo } from '../api/stations'
import './GameCanvas.css'

// M7: World expanded to 8000x8000, centered at (4000, 4000)
const WORLD_SIZE = 8000
const WORLD_CENTER = 4000
const PLAYER_RADIUS = 8
const NODE_RADIUS = 12
const EXTRACTION_RANGE = 50
const STATION_INTERACTION_RANGE = 75
const TRADE_RANGE = 75
const STATION_SIZE = 20
const GOLDEN = '#c9a227'
const PURPLE = '#8b5cf6'

// Terrain colors
const WATER_COLOR = 'rgba(30, 100, 160, 0.7)'
const WATER_HIGHLIGHT = 'rgba(100, 180, 220, 0.4)'
const BRIDGE_COLOR = '#8b6914'
const BRIDGE_EDGE_COLOR = '#5d4612'
const ROAD_COLORS = {
  arterial: '#4a4a5a',
  grid: '#3a3a4a',
  suburban: '#2a2a3a',
  trail: '#1a1a2a',
}

// M7: Zone definitions with colors (radii scaled 4x for 8000x8000 world)
const ZONES = [
  { id: 'capitol', name: 'Capitol', innerRadius: 0, outerRadius: 200, color: '#c9a227', alpha: 0.15 },
  { id: 'trade_district', name: 'Trade District', innerRadius: 200, outerRadius: 600, color: '#3b82f6', alpha: 0.1 },
  { id: 'guild_district', name: 'Guild District', innerRadius: 600, outerRadius: 1000, color: '#8b5cf6', alpha: 0.1 },
  { id: 'urban', name: 'Urban', innerRadius: 1000, outerRadius: 1600, color: '#64748b', alpha: 0.08 },
  { id: 'suburban', name: 'Suburban', innerRadius: 1600, outerRadius: 2400, color: '#22c55e', alpha: 0.06 },
  { id: 'rural', name: 'Rural', innerRadius: 2400, outerRadius: 3400, color: '#84cc16', alpha: 0.04 },
  { id: 'wilderness', name: 'Wilderness', innerRadius: 3400, outerRadius: 4000, color: '#1a1a24', alpha: 0.0 },
]

// Debug camera constants
const DEBUG_PAN_SPEED = 400 // pixels per second
const DEBUG_ZOOM_MIN = 0.1
const DEBUG_ZOOM_MAX = 3.0
const DEBUG_ZOOM_STEP = 0.1

export default function GameCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const {
    players,
    nodes,
    stations,
    nearbyPlots,
    myPlayerId,
    isConnected,
    sendMove,
    startExtraction,
    moveToAndExtract,
    extractionState,
    pendingExtractionNodeId,
    craftingState,
    notifications,
    placementMode,
    updatePlacementPreview,
    placeStation,
    exitPlacementMode,
    openStationById,
    initiateTrade,
    selectPlot,
    terrain,
  } = useGame()

  const { isAdmin } = useAuth()

  // Debug camera state
  const [debugMode, setDebugMode] = useState(false)
  const [debugCameraX, setDebugCameraX] = useState(WORLD_CENTER)
  const [debugCameraY, setDebugCameraY] = useState(WORLD_CENTER)
  const [debugZoom, setDebugZoom] = useState(1.0)
  const keysPressed = useRef<Set<string>>(new Set())
  const lastFrameTime = useRef<number>(0)

  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, width: number, height: number) => {
      // Clear
      ctx.fillStyle = '#0a0a0f'
      ctx.fillRect(0, 0, width, height)

      // Get my player for camera
      const myPlayer = myPlayerId ? players.get(myPlayerId) : null

      // Use debug camera if enabled, otherwise follow player
      const zoom = debugMode ? debugZoom : 1.0
      const cameraX = debugMode ? debugCameraX : (myPlayer ? myPlayer.x : WORLD_CENTER)
      const cameraY = debugMode ? debugCameraY : (myPlayer ? myPlayer.y : WORLD_CENTER)

      // Calculate offset for zoom-adjusted coordinate system
      // With ctx.scale(zoom), drawing at (x,y) appears at (x*zoom, y*zoom)
      // We want: worldX*zoom - cameraX*zoom + width/2
      // Using offsetX = width/(2*zoom) - cameraX, then (worldX + offsetX)*zoom = worldX*zoom + width/2 - cameraX*zoom
      const offsetX = width / (2 * zoom) - cameraX
      const offsetY = height / (2 * zoom) - cameraY

      // Apply zoom transformation for world rendering
      ctx.save()
      ctx.scale(zoom, zoom)

      // M7: Draw zone rings (centered at WORLD_CENTER, WORLD_CENTER)
      const centerScreenX = WORLD_CENTER + offsetX
      const centerScreenY = WORLD_CENTER + offsetY

      ZONES.forEach((zone) => {
        if (zone.alpha > 0) {
          ctx.beginPath()
          ctx.arc(centerScreenX, centerScreenY, zone.outerRadius, 0, Math.PI * 2)
          ctx.arc(centerScreenX, centerScreenY, zone.innerRadius, 0, Math.PI * 2, true)
          ctx.fillStyle = zone.color + Math.round(zone.alpha * 255).toString(16).padStart(2, '0')
          ctx.fill()
        }

        // Draw zone boundary line
        ctx.beginPath()
        ctx.arc(centerScreenX, centerScreenY, zone.outerRadius, 0, Math.PI * 2)
        ctx.strokeStyle = zone.color + '40'
        ctx.lineWidth = 1
        ctx.stroke()
      })

      // Draw terrain (roads first so water covers road ends)
      if (terrain) {
        // Suburban plots (draw first, underneath roads)
        if (terrain.suburban_plots) {
          drawSuburbanPlots(ctx, offsetX, offsetY, terrain.suburban_plots)
        }
        // Roads
        if (terrain.roads) {
          drawRoads(ctx, centerScreenX, centerScreenY, offsetX, offsetY, terrain.roads)
        }
        // Water rings
        if (terrain.water_rings) {
          drawWaterRings(ctx, centerScreenX, centerScreenY, terrain.water_rings)
        }
        // Bridges
        if (terrain.water_rings) {
          drawBridges(ctx, centerScreenX, centerScreenY, terrain.water_rings)
        }
      }

      // Draw world boundary
      ctx.strokeStyle = GOLDEN
      ctx.lineWidth = 2
      ctx.strokeRect(offsetX, offsetY, WORLD_SIZE, WORLD_SIZE)

      // M7: Draw nearby plots
      nearbyPlots.forEach((plot) => {
        const plotScreenX = plot.worldX + offsetX
        const plotScreenY = plot.worldY + offsetY
        const halfWidth = (plot.bounds.maxX - plot.bounds.minX) / 2
        const halfHeight = (plot.bounds.maxY - plot.bounds.minY) / 2

        // Determine plot color based on ownership
        let plotColor = '#4a4a54' // unclaimed - gray
        let plotAlpha = 0.2
        if (plot.ownerId) {
          if (myPlayerId && plot.ownerId === myPlayerId) {
            plotColor = '#22c55e' // owned by me - green
            plotAlpha = 0.3
          } else {
            plotColor = '#ef4444' // owned by others - red
            plotAlpha = 0.2
          }
        }

        // Draw plot area
        ctx.fillStyle = plotColor + Math.round(plotAlpha * 255).toString(16).padStart(2, '0')
        ctx.fillRect(
          plotScreenX - halfWidth,
          plotScreenY - halfHeight,
          halfWidth * 2,
          halfHeight * 2
        )

        // Draw plot border
        ctx.strokeStyle = plotColor + '80'
        ctx.lineWidth = 1
        ctx.strokeRect(
          plotScreenX - halfWidth,
          plotScreenY - halfHeight,
          halfWidth * 2,
          halfHeight * 2
        )
      })

      // Draw resource nodes
      nodes.forEach((node) => {
        const screenX = node.x + offsetX
        const screenY = node.y + offsetY

        const isAvailable = node.state === 'available'
        const isHarvesting = node.state === 'harvesting'
        const isPendingTarget = node.id === pendingExtractionNodeId

        // Draw grass tuft shape
        drawGrassTuft(ctx, screenX, screenY, isAvailable, isHarvesting, isPendingTarget)
      })

      // Draw stations
      stations.forEach((station) => {
        const screenX = station.x + offsetX
        const screenY = station.y + offsetY

        // Check if player is in range
        let inRange = false
        if (myPlayer) {
          const dx = myPlayer.x - station.x
          const dy = myPlayer.y - station.y
          const distance = Math.sqrt(dx * dx + dy * dy)
          inRange = distance <= STATION_INTERACTION_RANGE
        }

        drawStation(ctx, screenX, screenY, station.station_type, inRange)
      })

      // Draw placement preview
      if (placementMode && placementMode.active && myPlayer) {
        const previewScreenX = placementMode.previewX + offsetX
        const previewScreenY = placementMode.previewY + offsetY

        // Check if valid placement (not too close to other stations)
        let validPlacement = true
        stations.forEach((station) => {
          const dx = placementMode.previewX - station.x
          const dy = placementMode.previewY - station.y
          const distance = Math.sqrt(dx * dx + dy * dy)
          if (distance < 50) {
            validPlacement = false
          }
        })

        // Check if in range of player
        const dx = myPlayer.x - placementMode.previewX
        const dy = myPlayer.y - placementMode.previewY
        const distanceToPlayer = Math.sqrt(dx * dx + dy * dy)
        if (distanceToPlayer > STATION_INTERACTION_RANGE) {
          validPlacement = false
        }

        // M7: Check if on owned plot
        let onOwnedPlot = false
        for (const plot of nearbyPlots) {
          if (
            placementMode.previewX >= plot.bounds.minX &&
            placementMode.previewX <= plot.bounds.maxX &&
            placementMode.previewY >= plot.bounds.minY &&
            placementMode.previewY <= plot.bounds.maxY &&
            plot.ownerId === myPlayerId
          ) {
            onOwnedPlot = true
            break
          }
        }
        if (!onOwnedPlot) {
          validPlacement = false
        }

        drawStationPreview(ctx, previewScreenX, previewScreenY, placementMode.stationType, validPlacement)
      }

      // Draw line to pending extraction target
      if (pendingExtractionNodeId && myPlayer) {
        const targetNode = nodes.get(pendingExtractionNodeId)
        if (targetNode) {
          const playerScreenX = myPlayer.x + offsetX
          const playerScreenY = myPlayer.y + offsetY
          const targetScreenX = targetNode.x + offsetX
          const targetScreenY = targetNode.y + offsetY

          // Dashed line from player to target
          ctx.beginPath()
          ctx.setLineDash([5, 5])
          ctx.moveTo(playerScreenX, playerScreenY)
          ctx.lineTo(targetScreenX, targetScreenY)
          ctx.strokeStyle = 'rgba(201, 162, 39, 0.5)'
          ctx.lineWidth = 2
          ctx.stroke()
          ctx.setLineDash([])
        }
      }

      // Draw players
      players.forEach((player) => {
        const screenX = player.x + offsetX
        const screenY = player.y + offsetY

        const isMe = player.id === myPlayerId

        // Player circle
        ctx.beginPath()
        ctx.arc(screenX, screenY, PLAYER_RADIUS, 0, Math.PI * 2)
        ctx.fillStyle = isMe ? GOLDEN : '#4a7c59'
        ctx.fill()
        ctx.strokeStyle = isMe ? '#ffffff' : '#2d4e36'
        ctx.lineWidth = 2
        ctx.stroke()

        // Draw extraction progress ring for extracting players
        if (isMe && extractionState) {
          const progress = extractionState.progress / extractionState.duration
          drawExtractionRing(ctx, screenX, screenY, progress)
        }

        // Draw crafting progress ring
        if (isMe && craftingState) {
          const progress = craftingState.progress / craftingState.duration
          drawCraftingRing(ctx, screenX, screenY, progress)
        }

        // Player name
        ctx.font = '12px Segoe UI'
        ctx.textAlign = 'center'
        ctx.fillStyle = '#e0e0e0'
        ctx.fillText(player.name, screenX, screenY - PLAYER_RADIUS - 6)
      })

      // Restore non-zoomed coordinate system for UI elements
      ctx.restore()

      // Draw debug mode indicator
      if (debugMode) {
        ctx.font = '14px Segoe UI'
        ctx.textAlign = 'left'
        ctx.fillStyle = '#ef4444'
        ctx.fillText(`DEBUG MODE - Zoom: ${zoom.toFixed(2)}x`, 20, 30)
        ctx.fillText(`Camera: (${Math.round(cameraX)}, ${Math.round(cameraY)})`, 20, 50)
        ctx.fillStyle = '#888'
        ctx.font = '12px Segoe UI'
        ctx.fillText('WASD/Arrows: Pan | Scroll: Zoom | F1: Exit', 20, 70)
      }

      // Draw notifications
      if (notifications.length > 0) {
        ctx.font = '14px Segoe UI'
        ctx.textAlign = 'right'
        notifications.forEach((notification, index) => {
          const y = (debugMode ? 90 : 30) + index * 24
          ctx.fillStyle = GOLDEN
          ctx.fillText(notification.message, width - 20, y)
        })
      }

      // Connection status
      if (!isConnected) {
        ctx.fillStyle = 'rgba(0, 0, 0, 0.7)'
        ctx.fillRect(0, 0, width, height)
        ctx.font = '20px Segoe UI'
        ctx.textAlign = 'center'
        ctx.fillStyle = GOLDEN
        ctx.fillText('Connecting...', width / 2, height / 2)
      }
    },
    [players, nodes, stations, nearbyPlots, myPlayerId, isConnected, extractionState, pendingExtractionNodeId, craftingState, notifications, placementMode, terrain, debugMode, debugCameraX, debugCameraY, debugZoom]
  )

  // Draw a grass tuft shape
  function drawGrassTuft(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    isAvailable: boolean,
    isHarvesting: boolean,
    isPendingTarget: boolean = false
  ) {
    const baseColor = isAvailable ? '#4a7c59' : '#3a3a44'
    const tipColor = isAvailable ? '#6ba377' : '#4a4a54'
    const highlightColor = isHarvesting || isPendingTarget ? GOLDEN : null

    // Draw targeting ring for pending extraction
    if (isPendingTarget) {
      ctx.beginPath()
      ctx.arc(x, y, NODE_RADIUS + 4, 0, Math.PI * 2)
      ctx.strokeStyle = GOLDEN
      ctx.lineWidth = 2
      ctx.stroke()
    }

    // Draw multiple grass blades
    const blades = [
      { angle: -30, height: 14 },
      { angle: -10, height: 18 },
      { angle: 10, height: 16 },
      { angle: 30, height: 12 },
      { angle: 0, height: 20 },
    ]

    blades.forEach(({ angle, height }) => {
      const radians = (angle * Math.PI) / 180
      const tipX = x + Math.sin(radians) * height
      const tipY = y - height

      ctx.beginPath()
      ctx.moveTo(x - 2, y)
      ctx.quadraticCurveTo(x, y - height / 2, tipX, tipY)
      ctx.quadraticCurveTo(x, y - height / 2, x + 2, y)
      ctx.fillStyle = baseColor
      ctx.fill()

      // Highlight tip
      ctx.beginPath()
      ctx.arc(tipX, tipY, 2, 0, Math.PI * 2)
      ctx.fillStyle = highlightColor || tipColor
      ctx.fill()
    })

    // Base mound
    ctx.beginPath()
    ctx.ellipse(x, y + 2, 8, 4, 0, 0, Math.PI * 2)
    ctx.fillStyle = isAvailable ? '#3d5e42' : '#2a2a34'
    ctx.fill()
  }

  // Draw extraction progress ring
  function drawExtractionRing(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    progress: number
  ) {
    const radius = PLAYER_RADIUS + 6
    const startAngle = -Math.PI / 2
    const endAngle = startAngle + progress * Math.PI * 2

    // Background ring
    ctx.beginPath()
    ctx.arc(x, y, radius, 0, Math.PI * 2)
    ctx.strokeStyle = 'rgba(201, 162, 39, 0.3)'
    ctx.lineWidth = 3
    ctx.stroke()

    // Progress ring
    ctx.beginPath()
    ctx.arc(x, y, radius, startAngle, endAngle)
    ctx.strokeStyle = GOLDEN
    ctx.lineWidth = 3
    ctx.stroke()
  }

  // Draw a station
  function drawStation(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    stationType: string,
    inRange: boolean
  ) {
    const size = STATION_SIZE

    // Highlight ring if in range
    if (inRange) {
      ctx.beginPath()
      ctx.arc(x, y, size + 4, 0, Math.PI * 2)
      ctx.strokeStyle = 'rgba(249, 115, 22, 0.5)'
      ctx.lineWidth = 2
      ctx.stroke()
    }

    // Draw based on station type
    switch (stationType) {
      case 'workbench':
        // Table shape
        ctx.fillStyle = '#8b5a2b'
        ctx.fillRect(x - size / 2, y - size / 4, size, size / 2)
        // Legs
        ctx.fillStyle = '#5d3a1a'
        ctx.fillRect(x - size / 2 + 2, y + size / 4 - 2, 4, 6)
        ctx.fillRect(x + size / 2 - 6, y + size / 4 - 2, 4, 6)
        // Top highlight
        ctx.fillStyle = '#a67c52'
        ctx.fillRect(x - size / 2 + 2, y - size / 4, size - 4, 3)
        break

      case 'forge':
        // Furnace shape
        ctx.fillStyle = '#4a4a54'
        ctx.beginPath()
        ctx.moveTo(x - size / 2, y + size / 3)
        ctx.lineTo(x - size / 3, y - size / 3)
        ctx.lineTo(x + size / 3, y - size / 3)
        ctx.lineTo(x + size / 2, y + size / 3)
        ctx.closePath()
        ctx.fill()
        // Fire glow
        ctx.fillStyle = '#f97316'
        ctx.beginPath()
        ctx.arc(x, y, size / 4, 0, Math.PI * 2)
        ctx.fill()
        // Flame
        ctx.fillStyle = '#fbbf24'
        ctx.beginPath()
        ctx.arc(x, y - 2, size / 6, 0, Math.PI * 2)
        ctx.fill()
        break

      case 'storage_chest':
        // Chest box
        ctx.fillStyle = '#8b6914'
        ctx.fillRect(x - size / 2, y - size / 4, size, size / 2)
        // Lid
        ctx.fillStyle = '#a67c1a'
        ctx.fillRect(x - size / 2 - 1, y - size / 4 - 4, size + 2, 5)
        // Lock
        ctx.fillStyle = '#ffd700'
        ctx.beginPath()
        ctx.arc(x, y, 3, 0, Math.PI * 2)
        ctx.fill()
        break

      default:
        // Generic station
        ctx.fillStyle = '#5a5a6a'
        ctx.fillRect(x - size / 2, y - size / 2, size, size)
    }
  }

  // Draw station placement preview
  function drawStationPreview(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    stationType: string,
    valid: boolean
  ) {
    ctx.globalAlpha = 0.6
    ctx.strokeStyle = valid ? '#22c55e' : '#ef4444'
    ctx.lineWidth = 3

    // Draw preview ring
    ctx.beginPath()
    ctx.arc(x, y, STATION_SIZE + 6, 0, Math.PI * 2)
    ctx.stroke()

    // Draw station shape
    drawStation(ctx, x, y, stationType, false)

    ctx.globalAlpha = 1.0
  }

  // Draw crafting progress ring (purple)
  function drawCraftingRing(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    progress: number
  ) {
    const radius = PLAYER_RADIUS + 6
    const startAngle = -Math.PI / 2
    const endAngle = startAngle + progress * Math.PI * 2

    // Background ring
    ctx.beginPath()
    ctx.arc(x, y, radius, 0, Math.PI * 2)
    ctx.strokeStyle = 'rgba(139, 92, 246, 0.3)'
    ctx.lineWidth = 3
    ctx.stroke()

    // Progress ring
    ctx.beginPath()
    ctx.arc(x, y, radius, startAngle, endAngle)
    ctx.strokeStyle = PURPLE
    ctx.lineWidth = 3
    ctx.stroke()
  }

  // Draw suburban plots as polygons
  function drawSuburbanPlots(
    ctx: CanvasRenderingContext2D,
    offsetX: number,
    offsetY: number,
    plots: { id: string; corners: [[number, number], [number, number], [number, number], [number, number]]; area: number }[]
  ) {
    const PLOT_FILL_COLOR = 'rgba(34, 197, 94, 0.08)' // Subtle green
    const PLOT_STROKE_COLOR = 'rgba(34, 197, 94, 0.25)' // Green border

    for (const plot of plots) {
      const corners = plot.corners

      // Draw filled polygon
      ctx.beginPath()
      ctx.moveTo(corners[0][0] + offsetX, corners[0][1] + offsetY)
      for (let i = 1; i < corners.length; i++) {
        ctx.lineTo(corners[i][0] + offsetX, corners[i][1] + offsetY)
      }
      ctx.closePath()
      ctx.fillStyle = PLOT_FILL_COLOR
      ctx.fill()

      // Draw border
      ctx.strokeStyle = PLOT_STROKE_COLOR
      ctx.lineWidth = 1
      ctx.stroke()
    }
  }

  // Draw roads from terrain data
  function drawRoads(
    ctx: CanvasRenderingContext2D,
    _centerX: number,
    _centerY: number,
    offsetX: number,
    offsetY: number,
    roads: { id: string; road_type: string; points: [number, number][]; width: number }[]
  ) {
    // Sort by type to draw in order: trails -> suburban -> grid -> arterial
    const roadOrder = ['trail', 'suburban', 'grid', 'arterial']

    const sortedRoads = [...roads].sort((a, b) => {
      return roadOrder.indexOf(a.road_type) - roadOrder.indexOf(b.road_type)
    })

    for (const road of sortedRoads) {
      if (!road.points || road.points.length < 2) continue

      const color = ROAD_COLORS[road.road_type as keyof typeof ROAD_COLORS] || ROAD_COLORS.trail
      ctx.strokeStyle = color
      ctx.lineWidth = road.width
      ctx.lineCap = 'round'
      ctx.lineJoin = 'round'

      ctx.beginPath()
      const [firstX, firstY] = road.points[0]
      ctx.moveTo(firstX + offsetX, firstY + offsetY)

      for (let i = 1; i < road.points.length; i++) {
        const [x, y] = road.points[i]
        ctx.lineTo(x + offsetX, y + offsetY)
      }
      ctx.stroke()
    }
  }

  // Draw water rings (annular water features with bridge gaps)
  function drawWaterRings(
    ctx: CanvasRenderingContext2D,
    centerX: number,
    centerY: number,
    waterRings: { id: string; radius: number; width: number; bridges: { angle_degrees: number; width: number }[] }[]
  ) {
    const segments = 64

    for (const ring of waterRings) {
      const innerRadius = ring.radius - ring.width / 2
      const outerRadius = ring.radius + ring.width / 2
      const segmentAngle = (Math.PI * 2) / segments

      // Calculate bridge angular spans
      const bridgeSpans: { start: number; finish: number }[] = []
      for (const bridge of ring.bridges) {
        const halfWidthRad = bridge.width / ring.radius
        const centerRad = (bridge.angle_degrees * Math.PI) / 180
        bridgeSpans.push({
          start: centerRad - halfWidthRad,
          finish: centerRad + halfWidthRad,
        })
      }

      // Draw water segments
      ctx.fillStyle = WATER_COLOR

      for (let i = 0; i < segments; i++) {
        const angle1 = i * segmentAngle
        const angle2 = (i + 1) * segmentAngle
        const midAngle = (angle1 + angle2) / 2

        // Check if this segment overlaps with any bridge
        let onBridge = false
        for (const span of bridgeSpans) {
          let spanStart = span.start
          let spanFinish = span.finish

          // Normalize angles
          if (spanStart < 0) spanStart += Math.PI * 2
          if (spanFinish < 0) spanFinish += Math.PI * 2

          // Handle wraparound
          if (spanFinish < spanStart) {
            // Span crosses 0
            if (midAngle >= spanStart || midAngle <= spanFinish) {
              onBridge = true
              break
            }
          } else {
            if (midAngle >= spanStart && midAngle <= spanFinish) {
              onBridge = true
              break
            }
          }

          // Also check original span (negative angles)
          if (span.start < 0 && midAngle > Math.PI) {
            const normalizedMid = midAngle - Math.PI * 2
            if (normalizedMid >= span.start && normalizedMid <= span.finish) {
              onBridge = true
              break
            }
          }
        }

        if (!onBridge) {
          // Draw this water segment as a quad
          const x1Inner = centerX + Math.cos(angle1) * innerRadius
          const y1Inner = centerY + Math.sin(angle1) * innerRadius
          const x2Inner = centerX + Math.cos(angle2) * innerRadius
          const y2Inner = centerY + Math.sin(angle2) * innerRadius
          const x1Outer = centerX + Math.cos(angle1) * outerRadius
          const y1Outer = centerY + Math.sin(angle1) * outerRadius
          const x2Outer = centerX + Math.cos(angle2) * outerRadius
          const y2Outer = centerY + Math.sin(angle2) * outerRadius

          ctx.beginPath()
          ctx.moveTo(x1Inner, y1Inner)
          ctx.lineTo(x2Inner, y2Inner)
          ctx.lineTo(x2Outer, y2Outer)
          ctx.lineTo(x1Outer, y1Outer)
          ctx.closePath()
          ctx.fill()
        }
      }

      // Draw water highlight ring
      ctx.strokeStyle = WATER_HIGHLIGHT
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.arc(centerX, centerY, ring.radius, 0, Math.PI * 2)
      ctx.stroke()
    }
  }

  // Draw bridges
  function drawBridges(
    ctx: CanvasRenderingContext2D,
    centerX: number,
    centerY: number,
    waterRings: { id: string; radius: number; width: number; bridges: { angle_degrees: number; width: number }[] }[]
  ) {
    for (const ring of waterRings) {
      const innerRadius = ring.radius - ring.width / 2 - 2
      const outerRadius = ring.radius + ring.width / 2 + 2

      for (const bridge of ring.bridges) {
        const angleRad = (bridge.angle_degrees * Math.PI) / 180
        const halfWidthRad = (bridge.width / innerRadius) * 1.2

        const angle1 = angleRad - halfWidthRad
        const angle2 = angleRad + halfWidthRad

        // Calculate bridge corners
        const x1Inner = centerX + Math.cos(angle1) * innerRadius
        const y1Inner = centerY + Math.sin(angle1) * innerRadius
        const x2Inner = centerX + Math.cos(angle2) * innerRadius
        const y2Inner = centerY + Math.sin(angle2) * innerRadius
        const x1Outer = centerX + Math.cos(angle1) * outerRadius
        const y1Outer = centerY + Math.sin(angle1) * outerRadius
        const x2Outer = centerX + Math.cos(angle2) * outerRadius
        const y2Outer = centerY + Math.sin(angle2) * outerRadius

        // Draw bridge surface
        ctx.fillStyle = BRIDGE_COLOR
        ctx.beginPath()
        ctx.moveTo(x1Inner, y1Inner)
        ctx.lineTo(x2Inner, y2Inner)
        ctx.lineTo(x2Outer, y2Outer)
        ctx.lineTo(x1Outer, y1Outer)
        ctx.closePath()
        ctx.fill()

        // Draw bridge edges
        ctx.strokeStyle = BRIDGE_EDGE_COLOR
        ctx.lineWidth = 2
        ctx.beginPath()
        ctx.moveTo(x1Inner, y1Inner)
        ctx.lineTo(x1Outer, y1Outer)
        ctx.stroke()
        ctx.beginPath()
        ctx.moveTo(x2Inner, y2Inner)
        ctx.lineTo(x2Outer, y2Outer)
        ctx.stroke()
      }
    }
  }

  // Debug camera keyboard controls
  useEffect(() => {
    if (!isAdmin) return

    const handleKeyDown = (e: KeyboardEvent) => {
      // F1 toggles debug mode
      if (e.key === 'F1') {
        e.preventDefault()
        setDebugMode((prev) => {
          if (!prev) {
            // Entering debug mode: initialize camera at player position
            const myPlayer = myPlayerId ? players.get(myPlayerId) : null
            if (myPlayer) {
              setDebugCameraX(myPlayer.x)
              setDebugCameraY(myPlayer.y)
            }
          }
          return !prev
        })
        return
      }

      // Only handle pan keys in debug mode
      if (!debugMode) return

      if (['w', 'W', 'ArrowUp', 's', 'S', 'ArrowDown', 'a', 'A', 'ArrowLeft', 'd', 'D', 'ArrowRight'].includes(e.key)) {
        e.preventDefault()
        keysPressed.current.add(e.key.toLowerCase())
      }
    }

    const handleKeyUp = (e: KeyboardEvent) => {
      keysPressed.current.delete(e.key.toLowerCase())
      keysPressed.current.delete(e.key)
    }

    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('keyup', handleKeyUp)

    return () => {
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('keyup', handleKeyUp)
    }
  }, [isAdmin, debugMode, myPlayerId, players])

  // Debug camera zoom with mouse wheel
  useEffect(() => {
    if (!isAdmin || !debugMode) return

    const canvas = canvasRef.current
    if (!canvas) return

    const handleWheel = (e: WheelEvent) => {
      e.preventDefault()
      const delta = e.deltaY > 0 ? -DEBUG_ZOOM_STEP : DEBUG_ZOOM_STEP
      setDebugZoom((prev) => Math.max(DEBUG_ZOOM_MIN, Math.min(DEBUG_ZOOM_MAX, prev + delta)))
    }

    canvas.addEventListener('wheel', handleWheel, { passive: false })

    return () => {
      canvas.removeEventListener('wheel', handleWheel)
    }
  }, [isAdmin, debugMode])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const resize = () => {
      canvas.width = canvas.offsetWidth
      canvas.height = canvas.offsetHeight
      draw(ctx, canvas.width, canvas.height)
    }

    window.addEventListener('resize', resize)
    resize()

    let animationId: number
    const animate = (timestamp: number) => {
      // Handle debug camera panning
      if (debugMode && keysPressed.current.size > 0) {
        const dt = lastFrameTime.current > 0 ? (timestamp - lastFrameTime.current) / 1000 : 0
        const panSpeed = DEBUG_PAN_SPEED / debugZoom // Pan speed adjusts with zoom

        let dx = 0
        let dy = 0
        if (keysPressed.current.has('w') || keysPressed.current.has('arrowup')) dy -= panSpeed * dt
        if (keysPressed.current.has('s') || keysPressed.current.has('arrowdown')) dy += panSpeed * dt
        if (keysPressed.current.has('a') || keysPressed.current.has('arrowleft')) dx -= panSpeed * dt
        if (keysPressed.current.has('d') || keysPressed.current.has('arrowright')) dx += panSpeed * dt

        if (dx !== 0 || dy !== 0) {
          setDebugCameraX((prev) => Math.max(0, Math.min(WORLD_SIZE, prev + dx)))
          setDebugCameraY((prev) => Math.max(0, Math.min(WORLD_SIZE, prev + dy)))
        }
      }
      lastFrameTime.current = timestamp

      draw(ctx, canvas.width, canvas.height)
      animationId = requestAnimationFrame(animate)
    }
    animationId = requestAnimationFrame(animate)

    return () => {
      window.removeEventListener('resize', resize)
      cancelAnimationFrame(animationId)
    }
  }, [draw, debugMode, debugZoom])

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current
    console.log('Canvas click, myPlayerId:', myPlayerId)
    if (!canvas || !myPlayerId) return

    const rect = canvas.getBoundingClientRect()
    const clickX = e.clientX - rect.left
    const clickY = e.clientY - rect.top

    // Get my player for camera offset
    const myPlayer = players.get(myPlayerId)
    console.log('My player from map:', myPlayer)
    if (!myPlayer) return

    // Account for debug camera and zoom when converting coordinates
    const zoom = debugMode ? debugZoom : 1.0
    const cameraX = debugMode ? debugCameraX : myPlayer.x
    const cameraY = debugMode ? debugCameraY : myPlayer.y

    // Convert screen coords to world coords (accounting for zoom)
    // Screen position = (worldPos - cameraPos) * zoom + screenCenter
    // So: worldPos = (screenPos - screenCenter) / zoom + cameraPos
    const worldX = (clickX - canvas.width / 2) / zoom + cameraX
    const worldY = (clickY - canvas.height / 2) / zoom + cameraY

    // Handle placement mode
    if (placementMode && placementMode.active) {
      // Validate placement
      let validPlacement = true
      stations.forEach((station) => {
        const dx = worldX - station.x
        const dy = worldY - station.y
        const distance = Math.sqrt(dx * dx + dy * dy)
        if (distance < 50) {
          validPlacement = false
        }
      })

      const dx = myPlayer.x - worldX
      const dy = myPlayer.y - worldY
      const distanceToPlayer = Math.sqrt(dx * dx + dy * dy)
      if (distanceToPlayer > STATION_INTERACTION_RANGE) {
        validPlacement = false
      }

      // M7: Check if on owned plot
      let onOwnedPlot = false
      for (const plot of nearbyPlots) {
        if (
          worldX >= plot.bounds.minX &&
          worldX <= plot.bounds.maxX &&
          worldY >= plot.bounds.minY &&
          worldY <= plot.bounds.maxY &&
          plot.ownerId === myPlayerId
        ) {
          onOwnedPlot = true
          break
        }
      }
      if (!onOwnedPlot) {
        validPlacement = false
      }

      if (validPlacement) {
        const clampedX = Math.max(0, Math.min(WORLD_SIZE, worldX))
        const clampedY = Math.max(0, Math.min(WORLD_SIZE, worldY))
        placeStation(clampedX, clampedY)
      }
      return
    }

    // Check for Shift+click on another player to initiate trade
    if (e.shiftKey) {
      let clickedPlayer: PlayerPosition | null = null
      for (const player of players.values()) {
        if (player.id === myPlayerId) continue // Skip self
        const dx = worldX - player.x
        const dy = worldY - player.y
        const distance = Math.sqrt(dx * dx + dy * dy)
        if (distance <= PLAYER_RADIUS + 8) {
          clickedPlayer = player
          break
        }
      }

      if (clickedPlayer) {
        // Check if in trade range
        const dx = myPlayer.x - clickedPlayer.x
        const dy = myPlayer.y - clickedPlayer.y
        const distanceToPlayer = Math.sqrt(dx * dx + dy * dy)

        if (distanceToPlayer <= TRADE_RANGE) {
          console.log('Initiating trade with:', clickedPlayer.id)
          initiateTrade(clickedPlayer.id)
          return
        } else {
          console.log('Player too far to trade, moving closer')
          sendMove(clickedPlayer.x, clickedPlayer.y)
          return
        }
      }
    }

    // Check if clicking on a station
    let clickedStation: StationInfo | undefined = undefined
    stations.forEach((station) => {
      const dx = worldX - station.x
      const dy = worldY - station.y
      const distance = Math.sqrt(dx * dx + dy * dy)
      if (distance <= STATION_SIZE + 4) {
        clickedStation = station
      }
    })

    if (clickedStation) {
      const station = clickedStation as StationInfo

      // Check if in range
      const dx = myPlayer.x - station.x
      const dy = myPlayer.y - station.y
      const distanceToStation = Math.sqrt(dx * dx + dy * dy)

      if (distanceToStation <= STATION_INTERACTION_RANGE) {
        console.log('Opening station:', station.id)
        openStationById(station.id)
        return
      } else {
        console.log('Station too far, moving closer')
        sendMove(station.x, station.y)
        return
      }
    }

    // M7: Check if clicking on a plot (Alt+click to select)
    if (e.altKey) {
      for (const plot of nearbyPlots) {
        if (
          worldX >= plot.bounds.minX &&
          worldX <= plot.bounds.maxX &&
          worldY >= plot.bounds.minY &&
          worldY <= plot.bounds.maxY
        ) {
          console.log('Selected plot:', plot.id)
          selectPlot(plot)
          return
        }
      }
    }

    // Check if clicking on a resource node
    let clickedNode: ResourceNode | undefined = undefined
    nodes.forEach((node) => {
      const dx = worldX - node.x
      const dy = worldY - node.y
      const distance = Math.sqrt(dx * dx + dy * dy)
      if (distance <= NODE_RADIUS + 4) {
        clickedNode = node
      }
    })

    if (clickedNode !== undefined) {
      const node = clickedNode as ResourceNode

      // Only interact with available nodes
      if (node.state !== 'available') {
        console.log('Node is not available:', node.state)
        // Fall through to move to that location
      } else {
        // Check if in range
        const dx = myPlayer.x - node.x
        const dy = myPlayer.y - node.y
        const distanceToNode = Math.sqrt(dx * dx + dy * dy)

        if (distanceToNode <= EXTRACTION_RANGE) {
          console.log('Starting extraction on node:', node.id)
          startExtraction(node.id)
          return
        } else {
          // Move to node and extract when in range
          console.log('Moving to node for extraction:', node.id)
          moveToAndExtract(node.id)
          return
        }
      }
    }

    // Clamp to world bounds
    const clampedX = Math.max(0, Math.min(WORLD_SIZE, worldX))
    const clampedY = Math.max(0, Math.min(WORLD_SIZE, worldY))

    console.log('Sending move to:', clampedX, clampedY)
    sendMove(clampedX, clampedY)
  }

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!placementMode || !placementMode.active) return

    const canvas = canvasRef.current
    if (!canvas || !myPlayerId) return

    const rect = canvas.getBoundingClientRect()
    const clickX = e.clientX - rect.left
    const clickY = e.clientY - rect.top

    const myPlayer = players.get(myPlayerId)
    if (!myPlayer) return

    // Account for debug camera and zoom when converting coordinates
    const zoom = debugMode ? debugZoom : 1.0
    const cameraX = debugMode ? debugCameraX : myPlayer.x
    const cameraY = debugMode ? debugCameraY : myPlayer.y

    const worldX = (clickX - canvas.width / 2) / zoom + cameraX
    const worldY = (clickY - canvas.height / 2) / zoom + cameraY

    updatePlacementPreview(worldX, worldY)
  }

  const handleRightClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    e.preventDefault()
    if (placementMode && placementMode.active) {
      exitPlacementMode()
    }
  }

  return (
    <canvas
      ref={canvasRef}
      className={`game-canvas ${placementMode?.active ? 'placement-mode' : ''}`}
      onClick={handleClick}
      onMouseMove={handleMouseMove}
      onContextMenu={handleRightClick}
    />
  )
}

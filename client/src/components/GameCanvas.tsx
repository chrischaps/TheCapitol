import { useRef, useEffect, useCallback } from 'react'
import { useGame } from '../contexts/GameContext'
import type { ResourceNode } from '../contexts/GameContext'
import type { StationInfo } from '../api/stations'
import './GameCanvas.css'

const WORLD_SIZE = 1000
const PLAYER_RADIUS = 8
const GRID_SIZE = 100
const NODE_RADIUS = 12
const EXTRACTION_RANGE = 50
const STATION_INTERACTION_RANGE = 75
const STATION_SIZE = 20
const GOLDEN = '#c9a227'
const PURPLE = '#8b5cf6'

export default function GameCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const {
    players,
    nodes,
    stations,
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
  } = useGame()

  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, width: number, height: number) => {
      // Clear
      ctx.fillStyle = '#0a0a0f'
      ctx.fillRect(0, 0, width, height)

      // Get my player for camera
      const myPlayer = myPlayerId ? players.get(myPlayerId) : null
      const cameraX = myPlayer ? myPlayer.x : WORLD_SIZE / 2
      const cameraY = myPlayer ? myPlayer.y : WORLD_SIZE / 2

      // Calculate offset to center camera on player
      const offsetX = width / 2 - cameraX
      const offsetY = height / 2 - cameraY

      // Draw grid
      ctx.strokeStyle = '#1a1a24'
      ctx.lineWidth = 1
      for (let x = 0; x <= WORLD_SIZE; x += GRID_SIZE) {
        const screenX = x + offsetX
        ctx.beginPath()
        ctx.moveTo(screenX, 0)
        ctx.lineTo(screenX, height)
        ctx.stroke()
      }
      for (let y = 0; y <= WORLD_SIZE; y += GRID_SIZE) {
        const screenY = y + offsetY
        ctx.beginPath()
        ctx.moveTo(0, screenY)
        ctx.lineTo(width, screenY)
        ctx.stroke()
      }

      // Draw world boundary
      ctx.strokeStyle = GOLDEN
      ctx.lineWidth = 2
      ctx.strokeRect(offsetX, offsetY, WORLD_SIZE, WORLD_SIZE)

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

      // Draw notifications
      if (notifications.length > 0) {
        ctx.font = '14px Segoe UI'
        ctx.textAlign = 'right'
        notifications.forEach((notification, index) => {
          const y = 30 + index * 24
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
    [players, nodes, stations, myPlayerId, isConnected, extractionState, pendingExtractionNodeId, craftingState, notifications, placementMode]
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
    const animate = () => {
      draw(ctx, canvas.width, canvas.height)
      animationId = requestAnimationFrame(animate)
    }
    animate()

    return () => {
      window.removeEventListener('resize', resize)
      cancelAnimationFrame(animationId)
    }
  }, [draw])

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

    const offsetX = canvas.width / 2 - myPlayer.x
    const offsetY = canvas.height / 2 - myPlayer.y

    // Convert screen coords to world coords
    const worldX = clickX - offsetX
    const worldY = clickY - offsetY

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

      if (validPlacement) {
        const clampedX = Math.max(0, Math.min(WORLD_SIZE, worldX))
        const clampedY = Math.max(0, Math.min(WORLD_SIZE, worldY))
        placeStation(clampedX, clampedY)
      }
      return
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
        // TODO: Move to station and open when in range
        console.log('Station too far, moving closer')
        sendMove(station.x, station.y)
        return
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

    const offsetX = canvas.width / 2 - myPlayer.x
    const offsetY = canvas.height / 2 - myPlayer.y

    const worldX = clickX - offsetX
    const worldY = clickY - offsetY

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

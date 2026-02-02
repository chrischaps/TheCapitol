import { useRef, useEffect, useCallback } from 'react'
import { useGame, PlayerPosition } from '../contexts/GameContext'
import './GameCanvas.css'

const WORLD_SIZE = 1000
const PLAYER_RADIUS = 8
const GRID_SIZE = 100

export default function GameCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const { players, myPlayerId, isConnected, sendMove } = useGame()

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
      ctx.strokeStyle = '#c9a227'
      ctx.lineWidth = 2
      ctx.strokeRect(offsetX, offsetY, WORLD_SIZE, WORLD_SIZE)

      // Draw players
      players.forEach((player) => {
        const screenX = player.x + offsetX
        const screenY = player.y + offsetY

        const isMe = player.id === myPlayerId

        // Player circle
        ctx.beginPath()
        ctx.arc(screenX, screenY, PLAYER_RADIUS, 0, Math.PI * 2)
        ctx.fillStyle = isMe ? '#c9a227' : '#4a7c59'
        ctx.fill()
        ctx.strokeStyle = isMe ? '#ffffff' : '#2d4e36'
        ctx.lineWidth = 2
        ctx.stroke()

        // Player name
        ctx.font = '12px Segoe UI'
        ctx.textAlign = 'center'
        ctx.fillStyle = '#e0e0e0'
        ctx.fillText(player.name, screenX, screenY - PLAYER_RADIUS - 6)
      })

      // Connection status
      if (!isConnected) {
        ctx.fillStyle = 'rgba(0, 0, 0, 0.7)'
        ctx.fillRect(0, 0, width, height)
        ctx.font = '20px Segoe UI'
        ctx.textAlign = 'center'
        ctx.fillStyle = '#c9a227'
        ctx.fillText('Connecting...', width / 2, height / 2)
      }
    },
    [players, myPlayerId, isConnected]
  )

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
    if (!canvas || !myPlayerId) return

    const rect = canvas.getBoundingClientRect()
    const clickX = e.clientX - rect.left
    const clickY = e.clientY - rect.top

    // Get my player for camera offset
    const myPlayer = players.get(myPlayerId)
    if (!myPlayer) return

    const offsetX = canvas.width / 2 - myPlayer.x
    const offsetY = canvas.height / 2 - myPlayer.y

    // Convert screen coords to world coords
    const worldX = clickX - offsetX
    const worldY = clickY - offsetY

    // Clamp to world bounds
    const clampedX = Math.max(0, Math.min(WORLD_SIZE, worldX))
    const clampedY = Math.max(0, Math.min(WORLD_SIZE, worldY))

    sendMove(clampedX, clampedY)
  }

  return (
    <canvas
      ref={canvasRef}
      className="game-canvas"
      onClick={handleClick}
    />
  )
}

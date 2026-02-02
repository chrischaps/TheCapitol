import { createContext, useContext, useState, useEffect, ReactNode, useCallback } from 'react'
import { useAuth } from './AuthContext'

export interface PlayerPosition {
  id: string
  name: string
  x: number
  y: number
}

interface GameContextType {
  players: Map<string, PlayerPosition>
  myPlayerId: string | null
  isConnected: boolean
  sendMove: (x: number, y: number) => void
}

const GameContext = createContext<GameContextType | null>(null)

export function GameProvider({ children }: { children: ReactNode }) {
  const { token, playerId } = useAuth()
  const [players, setPlayers] = useState<Map<string, PlayerPosition>>(new Map())
  const [isConnected, setIsConnected] = useState(false)
  const [socket, setSocket] = useState<WebSocket | null>(null)

  useEffect(() => {
    if (!token) return

    const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const wsUrl = `${wsProtocol}//${window.location.host}/ws`
    const ws = new WebSocket(wsUrl)

    ws.onopen = () => {
      ws.send(JSON.stringify({ type: 'auth', token }))
    }

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data)

      switch (msg.type) {
        case 'auth_ok':
          setIsConnected(true)
          ws.send(JSON.stringify({ type: 'subscribe', channel: 'position' }))
          break

        case 'auth_error':
          console.error('WebSocket auth error:', msg.message)
          ws.close()
          break

        case 'positions':
          setPlayers((prev) => {
            const next = new Map(prev)
            for (const player of msg.players) {
              next.set(player.id, player)
            }
            return next
          })
          break

        case 'player_joined':
          setPlayers((prev) => {
            const next = new Map(prev)
            next.set(msg.player.id, msg.player)
            return next
          })
          break

        case 'player_left':
          setPlayers((prev) => {
            const next = new Map(prev)
            next.delete(msg.player_id)
            return next
          })
          break
      }
    }

    ws.onclose = () => {
      setIsConnected(false)
      setSocket(null)
    }

    ws.onerror = (err) => {
      console.error('WebSocket error:', err)
    }

    setSocket(ws)

    return () => {
      ws.close()
    }
  }, [token])

  const sendMove = useCallback(
    (x: number, y: number) => {
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: 'move', x, y }))
      }
    },
    [socket]
  )

  const value: GameContextType = {
    players,
    myPlayerId: playerId,
    isConnected,
    sendMove,
  }

  return <GameContext.Provider value={value}>{children}</GameContext.Provider>
}

export function useGame() {
  const context = useContext(GameContext)
  if (!context) {
    throw new Error('useGame must be used within GameProvider')
  }
  return context
}

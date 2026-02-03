import { createContext, useContext, useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from 'react'
import { useAuth } from './AuthContext'
import { getInventory, type InventoryItem } from '../api/player'
import { getNearby } from '../api/world'
import { getRecipes, type Recipe } from '../api/recipes'
import { getContainers, type ContainerState as ApiContainerState, type SlotItem as ApiSlotItem } from '../api/inventory'
import { getNearbyStations, getStationContainer, type StationInfo } from '../api/stations'

export interface PlayerPosition {
  id: string
  name: string
  x: number
  y: number
}

export interface ResourceNode {
  id: string
  resource_type: string
  x: number
  y: number
  state: string
}

export interface ExtractionState {
  nodeId: string
  progress: number
  duration: number
}

export interface CraftingState {
  operationId: string
  recipeId: string
  recipeName: string
  progress: number
  duration: number
}

export interface InventoryStack {
  itemType: string
  name: string
  totalQuantity: number
  avgQuality: number
  avgQualityGrade: string
  items: InventoryItem[]
}

export interface Notification {
  id: string
  message: string
  timestamp: number
}

// Slot-based container types
export interface SlotItem {
  itemId: string
  itemType: string
  itemName: string
  quality: number
  qualityGrade: string
  quantity: number
  slotIndex: number
}

export interface ContainerState {
  id: string
  containerType: string
  slotCount: number
  layoutColumns: number
  slots: Map<number, SlotItem>
}

// M4: Station types
export interface PlacementMode {
  active: boolean
  stationType: string
  itemId: string
  previewX: number
  previewY: number
}

export interface OpenStation {
  id: string
  stationType: string
  name: string
  containerId: string
}

interface GameContextType {
  players: Map<string, PlayerPosition>
  nodes: Map<string, ResourceNode>
  inventory: InventoryItem[]
  inventoryStacks: InventoryStack[]
  recipes: Recipe[]
  isExtracting: boolean
  extractionState: ExtractionState | null
  pendingExtractionNodeId: string | null
  isCrafting: boolean
  craftingState: CraftingState | null
  notifications: Notification[]
  myPlayerId: string | null
  isConnected: boolean
  // Slot-based inventory
  inventoryContainer: ContainerState | null
  craftingInputContainer: ContainerState | null
  craftingOutputContainer: ContainerState | null
  // M4: Stations
  stations: Map<string, StationInfo>
  openStation: OpenStation | null
  openStationContainer: ContainerState | null
  placementMode: PlacementMode | null
  // Actions
  sendMove: (x: number, y: number) => void
  startExtraction: (nodeId: string) => void
  moveToAndExtract: (nodeId: string) => void
  cancelExtraction: () => void
  startCrafting: (recipeId: string, inputItemIds: string[]) => void
  cancelCrafting: () => void
  refreshInventory: () => void
  refreshRecipes: () => void
  refreshContainers: () => void
  moveItem: (itemId: string, targetContainerId: string, targetSlot: number) => void
  // M4: Station actions
  refreshStations: () => void
  enterPlacementMode: (stationType: string, itemId: string) => void
  exitPlacementMode: () => void
  updatePlacementPreview: (x: number, y: number) => void
  placeStation: (x: number, y: number) => void
  removeStation: (stationId: string) => void
  openStationById: (stationId: string) => void
  closeStation: () => void
  craftAtStation: (stationId: string, recipeId: string, inputItemIds: string[]) => void
}

const GameContext = createContext<GameContextType | null>(null)

function qualityToGrade(quality: number): string {
  if (quality >= 90) return 'A'
  if (quality >= 75) return 'B'
  if (quality >= 55) return 'C'
  if (quality >= 35) return 'D'
  return 'F'
}

function apiSlotToSlotItem(slot: ApiSlotItem): SlotItem {
  return {
    itemId: slot.item_id,
    itemType: slot.item_type,
    itemName: slot.item_name,
    quality: slot.quality,
    qualityGrade: slot.quality_grade,
    quantity: slot.quantity,
    slotIndex: slot.slot_index,
  }
}

function apiContainerToState(container: ApiContainerState): ContainerState {
  const slots = new Map<number, SlotItem>()
  for (const slot of container.slots) {
    slots.set(slot.slot_index, apiSlotToSlotItem(slot))
  }
  return {
    id: container.id,
    containerType: container.container_type,
    slotCount: container.slot_count,
    layoutColumns: container.layout_columns,
    slots,
  }
}

export function GameProvider({ children }: { children: ReactNode }) {
  const { token, playerId } = useAuth()
  const [players, setPlayers] = useState<Map<string, PlayerPosition>>(new Map())
  const [nodes, setNodes] = useState<Map<string, ResourceNode>>(new Map())
  const [inventory, setInventory] = useState<InventoryItem[]>([])
  const [recipes, setRecipes] = useState<Recipe[]>([])
  const [extractionState, setExtractionState] = useState<ExtractionState | null>(null)
  const [pendingExtractionNodeId, setPendingExtractionNodeId] = useState<string | null>(null)
  const [craftingState, setCraftingState] = useState<CraftingState | null>(null)
  const [notifications, setNotifications] = useState<Notification[]>([])
  const [isConnected, setIsConnected] = useState(false)
  const socketRef = useRef<WebSocket | null>(null)

  // Slot-based containers
  const [inventoryContainer, setInventoryContainer] = useState<ContainerState | null>(null)
  const [craftingInputContainer, setCraftingInputContainer] = useState<ContainerState | null>(null)
  const [craftingOutputContainer, setCraftingOutputContainer] = useState<ContainerState | null>(null)

  // M4: Stations
  const [stations, setStations] = useState<Map<string, StationInfo>>(new Map())
  const [openStation, setOpenStation] = useState<OpenStation | null>(null)
  const [openStationContainer, setOpenStationContainer] = useState<ContainerState | null>(null)
  const [placementMode, setPlacementMode] = useState<PlacementMode | null>(null)

  const EXTRACTION_RANGE = 50
  const STATION_INTERACTION_RANGE = 75

  // Compute inventory stacks from inventory (legacy)
  const inventoryStacks = useMemo(() => {
    const stackMap = new Map<string, InventoryStack>()

    for (const item of inventory) {
      const existing = stackMap.get(item.item_type)
      if (existing) {
        existing.totalQuantity += item.quantity
        existing.items.push(item)
      } else {
        stackMap.set(item.item_type, {
          itemType: item.item_type,
          name: item.name,
          totalQuantity: item.quantity,
          avgQuality: 0,
          avgQualityGrade: 'C',
          items: [item],
        })
      }
    }

    // Calculate average quality for each stack
    for (const stack of stackMap.values()) {
      let totalQuality = 0
      let totalCount = 0
      for (const item of stack.items) {
        totalQuality += item.quality * item.quantity
        totalCount += item.quantity
      }
      stack.avgQuality = totalCount > 0 ? Math.round(totalQuality / totalCount) : 0
      stack.avgQualityGrade = qualityToGrade(stack.avgQuality)
    }

    return Array.from(stackMap.values())
  }, [inventory])

  useEffect(() => {
    if (!token) return

    // Connect directly to backend WebSocket (Vite proxy can be unreliable for WS)
    const wsUrl = 'ws://localhost:3000/ws'
    console.log('Connecting to WebSocket:', wsUrl)
    const ws = new WebSocket(wsUrl)
    socketRef.current = ws

    ws.onopen = () => {
      console.log('WebSocket connected, sending auth...')
      ws.send(JSON.stringify({ type: 'auth', token }))
    }

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data)
      console.log('WebSocket message:', msg.type)

      switch (msg.type) {
        case 'auth_ok':
          console.log('Auth successful, subscribing to positions...')
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

        // M2: Extraction events
        case 'extraction_started':
          if (msg.player_id === playerId) {
            setExtractionState({
              nodeId: msg.node_id,
              progress: 0,
              duration: msg.duration_ticks,
            })
          }
          break

        case 'extraction_progress':
          if (msg.player_id === playerId) {
            setExtractionState((prev) =>
              prev ? { ...prev, progress: msg.progress } : null
            )
          }
          break

        case 'extraction_completed':
          if (msg.player_id === playerId) {
            setExtractionState(null)
            // Add notification
            const notification: Notification = {
              id: `${Date.now()}`,
              message: `${msg.item_name} +${msg.quantity} (Quality: ${msg.quality})`,
              timestamp: Date.now(),
            }
            setNotifications((prev) => [...prev, notification])
            // Remove notification after 3 seconds
            setTimeout(() => {
              setNotifications((prev) =>
                prev.filter((n) => n.id !== notification.id)
              )
            }, 3000)
          }
          break

        case 'extraction_cancelled':
          if (msg.player_id === playerId) {
            setExtractionState(null)
          }
          break

        case 'node_depleted':
          setNodes((prev) => {
            const next = new Map(prev)
            const node = next.get(msg.node_id)
            if (node) {
              next.set(msg.node_id, { ...node, state: 'depleted' })
            }
            return next
          })
          break

        case 'node_regenerated':
          setNodes((prev) => {
            const next = new Map(prev)
            const node = next.get(msg.node_id)
            if (node) {
              next.set(msg.node_id, { ...node, state: 'available' })
            }
            return next
          })
          break

        case 'nearby_nodes':
          setNodes((prev) => {
            const next = new Map(prev)
            for (const node of msg.nodes) {
              next.set(node.id, node)
            }
            return next
          })
          break

        // M3: Crafting events
        case 'crafting_started':
          if (msg.player_id === playerId) {
            setCraftingState({
              operationId: msg.operation_id,
              recipeId: msg.recipe_id,
              recipeName: msg.recipe_name,
              progress: 0,
              duration: msg.duration_ticks,
            })
          }
          break

        case 'crafting_progress':
          if (msg.player_id === playerId) {
            setCraftingState((prev) =>
              prev ? { ...prev, progress: msg.progress } : null
            )
          }
          break

        case 'crafting_completed':
          if (msg.player_id === playerId) {
            setCraftingState(null)
            // Add notification
            const craftNotification: Notification = {
              id: `${Date.now()}`,
              message: `Crafted ${msg.item_name} x${msg.quantity} (Quality: ${msg.quality})`,
              timestamp: Date.now(),
            }
            setNotifications((prev) => [...prev, craftNotification])
            setTimeout(() => {
              setNotifications((prev) =>
                prev.filter((n) => n.id !== craftNotification.id)
              )
            }, 3000)
          }
          break

        case 'crafting_cancelled':
          if (msg.player_id === playerId) {
            setCraftingState(null)
          }
          break

        case 'crafting_failed':
          if (msg.player_id === playerId) {
            setCraftingState(null)
            const failNotification: Notification = {
              id: `${Date.now()}`,
              message: `Crafting failed: ${msg.reason}`,
              timestamp: Date.now(),
            }
            setNotifications((prev) => [...prev, failNotification])
            setTimeout(() => {
              setNotifications((prev) =>
                prev.filter((n) => n.id !== failNotification.id)
              )
            }, 3000)
          }
          break

        // M3.5: Inventory events
        case 'inventory_updated':
          if (msg.player_id === playerId) {
            // Refresh containers when inventory changes
            refreshContainersInternal()
          }
          break

        case 'item_moved':
        case 'items_merged':
          if (msg.player_id === playerId) {
            // Refresh containers after move/merge
            refreshContainersInternal()
          }
          break

        // M4: Station events
        case 'station_placed':
          setStations((prev) => {
            const next = new Map(prev)
            next.set(msg.station_id, {
              id: msg.station_id,
              station_type: msg.station_type,
              name: msg.name,
              category: msg.station_type === 'storage_chest' ? 'storage' : 'crafting',
              x: msg.x,
              y: msg.y,
              owner_id: msg.owner_id,
              container_id: msg.container_id,
              interaction_range: STATION_INTERACTION_RANGE,
            })
            return next
          })
          // Refresh inventory since kit item was consumed
          refreshInventory()
          refreshContainersInternal()
          break

        case 'station_removed':
          setStations((prev) => {
            const next = new Map(prev)
            next.delete(msg.station_id)
            return next
          })
          // Close if we had this station open
          if (openStation?.id === msg.station_id) {
            setOpenStation(null)
            setOpenStationContainer(null)
          }
          break

        case 'station_opened':
          if (msg.player_id === playerId) {
            setOpenStation({
              id: msg.station_id,
              stationType: msg.station_type,
              name: msg.name,
              containerId: msg.container_id,
            })
            // Fetch station container
            if (token) {
              getStationContainer(token, msg.station_id).then((container) => {
                const state = apiContainerToState({
                  id: container.id,
                  container_type: container.container_type,
                  slot_count: container.slot_count,
                  layout_columns: container.layout_columns,
                  slots: container.slots,
                })
                setOpenStationContainer(state)
              }).catch(console.error)
            }
          }
          break

        case 'station_closed':
          if (msg.player_id === playerId) {
            setOpenStation(null)
            setOpenStationContainer(null)
          }
          break

        case 'nearby_stations':
          setStations((prev) => {
            const next = new Map(prev)
            for (const station of msg.stations) {
              next.set(station.id, station)
            }
            return next
          })
          break
      }
    }

    ws.onclose = () => {
      // Only update state if this is still the current socket
      // (avoids race condition with React StrictMode double-mounting)
      if (socketRef.current === ws) {
        setIsConnected(false)
        socketRef.current = null
      }
    }

    ws.onerror = (err) => {
      console.error('WebSocket error:', err)
    }

    return () => {
      ws.close()
    }
  }, [token])

  const sendMove = useCallback(
    (x: number, y: number) => {
      const socket = socketRef.current
      console.log('sendMove called:', x, y, 'socket state:', socket?.readyState)
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: 'move', x, y }))
        // Clear pending extraction when manually moving
        setPendingExtractionNodeId(null)
      } else {
        console.warn('Cannot send move - socket not ready')
      }
    },
    []
  )

  const startExtraction = useCallback(
    (nodeId: string) => {
      const socket = socketRef.current
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: 'extract', node_id: nodeId }))
        // Clear pending since we're now extracting
        setPendingExtractionNodeId(null)
      }
    },
    []
  )

  const moveToAndExtract = useCallback(
    (nodeId: string) => {
      const node = nodes.get(nodeId)
      if (!node) return

      const socket = socketRef.current
      if (socket && socket.readyState === WebSocket.OPEN) {
        // Set pending extraction target
        setPendingExtractionNodeId(nodeId)
        // Move towards the node
        socket.send(JSON.stringify({ type: 'move', x: node.x, y: node.y }))
        console.log('Moving to node for extraction:', nodeId)
      }
    },
    [nodes]
  )

  const cancelExtraction = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'cancel_extraction' }))
    }
    setPendingExtractionNodeId(null)
  }, [])

  const startCrafting = useCallback(
    (recipeId: string, inputItemIds: string[]) => {
      const socket = socketRef.current
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({
          type: 'craft',
          recipe_id: recipeId,
          input_item_ids: inputItemIds,
        }))
      }
    },
    []
  )

  const cancelCrafting = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'cancel_crafting' }))
    }
  }, [])

  const moveItem = useCallback(
    (itemId: string, targetContainerId: string, targetSlot: number) => {
      const socket = socketRef.current
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({
          type: 'move_item',
          item_id: itemId,
          target_container_id: targetContainerId,
          target_slot: targetSlot,
        }))
      }
    },
    []
  )

  const refreshInventory = useCallback(async () => {
    if (!token) return
    try {
      const response = await getInventory(token)
      setInventory(response.items)
    } catch (error) {
      console.error('Failed to refresh inventory:', error)
    }
  }, [token])

  const refreshNearbyNodes = useCallback(async () => {
    if (!token) return
    try {
      const response = await getNearby(token)
      setNodes((prev) => {
        const next = new Map(prev)
        for (const node of response.nodes) {
          next.set(node.id, node)
        }
        return next
      })
    } catch (error) {
      console.error('Failed to refresh nearby nodes:', error)
    }
  }, [token])

  const refreshRecipes = useCallback(async () => {
    if (!token) return
    try {
      const response = await getRecipes(token)
      setRecipes(response.recipes)
    } catch (error) {
      console.error('Failed to refresh recipes:', error)
    }
  }, [token])

  const refreshContainersInternal = useCallback(async () => {
    if (!token) return
    try {
      const response = await getContainers(token)
      for (const container of response.containers) {
        const state = apiContainerToState(container)
        switch (container.container_type) {
          case 'player_inventory':
            setInventoryContainer(state)
            break
          case 'crafting_input':
            setCraftingInputContainer(state)
            break
          case 'crafting_output':
            setCraftingOutputContainer(state)
            break
        }
      }
    } catch (error) {
      console.error('Failed to refresh containers:', error)
    }
  }, [token])

  const refreshContainers = refreshContainersInternal

  // M4: Station functions
  const refreshStations = useCallback(async () => {
    if (!token) return
    try {
      const response = await getNearbyStations(token)
      setStations((prev) => {
        const next = new Map(prev)
        for (const station of response.stations) {
          next.set(station.id, station)
        }
        return next
      })
    } catch (error) {
      console.error('Failed to refresh stations:', error)
    }
  }, [token])

  const enterPlacementMode = useCallback((stationType: string, itemId: string) => {
    setPlacementMode({
      active: true,
      stationType,
      itemId,
      previewX: 0,
      previewY: 0,
    })
  }, [])

  const exitPlacementMode = useCallback(() => {
    setPlacementMode(null)
  }, [])

  const updatePlacementPreview = useCallback((x: number, y: number) => {
    setPlacementMode((prev) => prev ? { ...prev, previewX: x, previewY: y } : null)
  }, [])

  const placeStation = useCallback((x: number, y: number) => {
    if (!placementMode) return
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'place_station',
        station_type: placementMode.stationType,
        x,
        y,
        kit_item_id: placementMode.itemId,
      }))
      setPlacementMode(null)
    }
  }, [placementMode])

  const removeStation = useCallback((stationId: string) => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'remove_station',
        station_id: stationId,
      }))
    }
  }, [])

  const openStationById = useCallback((stationId: string) => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'open_station',
        station_id: stationId,
      }))
    }
  }, [])

  const closeStation = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'close_station' }))
    }
    setOpenStation(null)
    setOpenStationContainer(null)
  }, [])

  const craftAtStation = useCallback((stationId: string, recipeId: string, inputItemIds: string[]) => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'craft_at_station',
        station_id: stationId,
        recipe_id: recipeId,
        input_item_ids: inputItemIds,
      }))
    }
  }, [])

  // Load inventory, nearby nodes, recipes, containers, and stations when connected
  useEffect(() => {
    if (isConnected && token) {
      refreshInventory()
      refreshNearbyNodes()
      refreshRecipes()
      refreshContainers()
      refreshStations()
    }
  }, [isConnected, token, refreshInventory, refreshNearbyNodes, refreshRecipes, refreshContainers, refreshStations])

  // Periodically refresh nearby nodes and stations (every 5 seconds)
  useEffect(() => {
    if (!isConnected || !token) return
    const interval = setInterval(() => {
      refreshNearbyNodes()
      refreshStations()
    }, 5000)
    return () => clearInterval(interval)
  }, [isConnected, token, refreshNearbyNodes, refreshStations])

  // Refresh inventory and containers after extraction/crafting completes
  useEffect(() => {
    if (notifications.length > 0) {
      refreshInventory()
      refreshContainers()
    }
  }, [notifications.length, refreshInventory, refreshContainers])

  // Check if player has arrived at pending extraction target
  useEffect(() => {
    if (!pendingExtractionNodeId || !playerId) return

    const myPlayer = players.get(playerId)
    const targetNode = nodes.get(pendingExtractionNodeId)

    if (!myPlayer || !targetNode) return

    // Check if node is still available
    if (targetNode.state !== 'available') {
      console.log('Pending extraction node no longer available')
      setPendingExtractionNodeId(null)
      return
    }

    // Check distance
    const dx = myPlayer.x - targetNode.x
    const dy = myPlayer.y - targetNode.y
    const distance = Math.sqrt(dx * dx + dy * dy)

    if (distance <= EXTRACTION_RANGE) {
      console.log('Arrived at node, starting extraction:', pendingExtractionNodeId)
      startExtraction(pendingExtractionNodeId)
    }
  }, [players, nodes, pendingExtractionNodeId, playerId, startExtraction])

  const value: GameContextType = {
    players,
    nodes,
    inventory,
    inventoryStacks,
    recipes,
    isExtracting: extractionState !== null,
    extractionState,
    pendingExtractionNodeId,
    isCrafting: craftingState !== null,
    craftingState,
    notifications,
    myPlayerId: playerId,
    isConnected,
    inventoryContainer,
    craftingInputContainer,
    craftingOutputContainer,
    // M4: Stations
    stations,
    openStation,
    openStationContainer,
    placementMode,
    sendMove,
    startExtraction,
    moveToAndExtract,
    cancelExtraction,
    startCrafting,
    cancelCrafting,
    refreshInventory,
    refreshRecipes,
    refreshContainers,
    moveItem,
    // M4: Station actions
    refreshStations,
    enterPlacementMode,
    exitPlacementMode,
    updatePlacementPreview,
    placeStation,
    removeStation,
    openStationById,
    closeStation,
    craftAtStation,
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

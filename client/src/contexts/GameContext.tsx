import { createContext, useContext, useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from 'react'
import { useAuth } from './AuthContext'
import { getInventory, type InventoryItem } from '../api/player'
import { getNearby } from '../api/world'
import { getRecipes, type Recipe } from '../api/recipes'
import { getContainers, type ContainerState as ApiContainerState, type SlotItem as ApiSlotItem } from '../api/inventory'
import { getNearbyStations, getStationContainer, type StationInfo } from '../api/stations'
import { getCurrency } from '../api/currency'
import { getPlots, claimPlot as claimPlotApi } from '../api/plots'
import { getTerrain, type TerrainData } from '../api/terrain'

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

// M5: Trading types
export interface TradeRequest {
  fromPlayer: string
  fromPlayerName: string
}

export interface TradeOffer {
  items: TradeOfferItem[]
  strands: number
}

export interface TradeOfferItem {
  itemId: string
  itemType: string
  itemName: string
  quantity: number
  quality: number
}

export interface ActiveTrade {
  tradeId: string
  partnerId: string
  partnerName: string
  myOffer: TradeOffer
  theirOffer: TradeOffer
  iAccepted: boolean
  theyAccepted: boolean
}

// M7: Zone types
export interface ZoneInfo {
  id: string
  name: string
}

// M7: Plot types (re-export from api)
export interface PlotInfo {
  id: string
  zoneId: string
  worldX: number
  worldY: number
  bounds: {
    minX: number
    minY: number
    maxX: number
    maxY: number
  }
  sizeCategory: string
  plotType: string
  ownerId: string | null
  ownerName: string | null
  claimedAt: string | null
  assessedValue: number
  stationCount: number
  stationCapacity: number
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
  // M5: Economy
  strandBalance: number
  // M5: Trading
  tradeRequest: TradeRequest | null
  activeTrade: ActiveTrade | null
  // M7: Zones and Plots
  currentZone: ZoneInfo | null
  nearbyPlots: PlotInfo[]
  selectedPlot: PlotInfo | null
  // Terrain
  terrain: TerrainData | null
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
  refreshCurrency: () => void
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
  // M5: Trading actions
  initiateTrade: (targetPlayerId: string) => void
  acceptTradeRequest: () => void
  declineTradeRequest: () => void
  updateTradeOffer: (items: { itemId: string; quantity: number }[], strands: number) => void
  acceptTrade: () => void
  cancelTrade: () => void
  // M7: Plot actions
  selectPlot: (plot: PlotInfo) => void
  closePlotPanel: () => void
  claimPlot: (plotId: string) => void
  refreshPlots: () => void
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

  // M5: Economy
  const [strandBalance, setStrandBalance] = useState<number>(0)

  // M5: Trading
  const [tradeRequest, setTradeRequest] = useState<TradeRequest | null>(null)
  const [activeTrade, setActiveTrade] = useState<ActiveTrade | null>(null)
  const activeTradeRef = useRef<ActiveTrade | null>(null)

  // M7: Zones and Plots
  const [currentZone, setCurrentZone] = useState<ZoneInfo | null>(null)
  const [nearbyPlots, setNearbyPlots] = useState<PlotInfo[]>([])
  const [selectedPlot, setSelectedPlot] = useState<PlotInfo | null>(null)

  // Terrain
  const [terrain, setTerrain] = useState<TerrainData | null>(null)

  const EXTRACTION_RANGE = 50
  const STATION_INTERACTION_RANGE = 75

  // Keep activeTradeRef in sync with activeTrade state for WebSocket handler closure
  useEffect(() => {
    activeTradeRef.current = activeTrade
  }, [activeTrade])

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

        // M5: Currency events
        case 'currency_changed':
          setStrandBalance(msg.new_balance)
          break

        // M7: Zone events
        case 'zone_changed':
          if (msg.player_id === playerId) {
            setCurrentZone({
              id: msg.to_zone,
              name: msg.zone_name,
            })
            // Add notification
            const zoneNotification: Notification = {
              id: `${Date.now()}`,
              message: `Entered ${msg.zone_name}`,
              timestamp: Date.now(),
            }
            setNotifications((prev) => [...prev, zoneNotification])
            setTimeout(() => {
              setNotifications((prev) => prev.filter((n) => n.id !== zoneNotification.id))
            }, 3000)
            // Refresh nearby plots when zone changes
            refreshPlotsInternal()
          }
          break

        // M5: Trading events
        case 'trade_requested':
          setTradeRequest({
            fromPlayer: msg.from_player,
            fromPlayerName: msg.from_player_name,
          })
          break

        case 'trade_request_declined':
          // The initiator gets notified their request was declined
          const declineNotification: Notification = {
            id: `${Date.now()}`,
            message: 'Trade request declined',
            timestamp: Date.now(),
          }
          setNotifications((prev) => [...prev, declineNotification])
          setTimeout(() => {
            setNotifications((prev) => prev.filter((n) => n.id !== declineNotification.id))
          }, 3000)
          break

        case 'trade_started':
          setTradeRequest(null)
          setActiveTrade({
            tradeId: msg.trade_id,
            partnerId: msg.partner_id,
            partnerName: msg.partner_name,
            myOffer: { items: [], strands: 0 },
            theirOffer: { items: [], strands: 0 },
            iAccepted: false,
            theyAccepted: false,
          })
          break

        case 'trade_offer_updated':
          // Use ref to get current activeTrade value (avoids stale closure)
          if (activeTradeRef.current && msg.trade_id === activeTradeRef.current.tradeId) {
            const isMyOffer = msg.player_id === playerId
            const offerItems: TradeOfferItem[] = msg.items.map((item: { item_id: string; item_type: string; item_name: string; quantity: number; quality: number }) => ({
              itemId: item.item_id,
              itemType: item.item_type,
              itemName: item.item_name,
              quantity: item.quantity,
              quality: item.quality,
            }))

            setActiveTrade((prev) => {
              if (!prev) return null
              return {
                ...prev,
                ...(isMyOffer
                  ? { myOffer: { items: offerItems, strands: msg.strands } }
                  : { theirOffer: { items: offerItems, strands: msg.strands } }),
                // Reset acceptance when offer changes
                iAccepted: false,
                theyAccepted: false,
              }
            })
          }
          break

        case 'trade_accepted':
          // Use ref to get current activeTrade value (avoids stale closure)
          if (activeTradeRef.current && msg.trade_id === activeTradeRef.current.tradeId) {
            const isMe = msg.player_id === playerId
            setActiveTrade((prev) => {
              if (!prev) return null
              return {
                ...prev,
                ...(isMe ? { iAccepted: true } : { theyAccepted: true }),
              }
            })
          }
          break

        case 'trade_executed':
          setActiveTrade(null)
          const tradeNotification: Notification = {
            id: `${Date.now()}`,
            message: 'Trade completed!',
            timestamp: Date.now(),
          }
          setNotifications((prev) => [...prev, tradeNotification])
          setTimeout(() => {
            setNotifications((prev) => prev.filter((n) => n.id !== tradeNotification.id))
          }, 3000)
          // Refresh inventory and currency
          refreshInventory()
          refreshContainersInternal()
          refreshCurrencyInternal()
          break

        case 'trade_cancelled':
          setActiveTrade(null)
          const cancelNotification: Notification = {
            id: `${Date.now()}`,
            message: `Trade cancelled: ${msg.reason}`,
            timestamp: Date.now(),
          }
          setNotifications((prev) => [...prev, cancelNotification])
          setTimeout(() => {
            setNotifications((prev) => prev.filter((n) => n.id !== cancelNotification.id))
          }, 3000)
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

  const refreshCurrencyInternal = useCallback(async () => {
    if (!token) return
    try {
      const response = await getCurrency(token)
      setStrandBalance(response.strand_balance)
    } catch (error) {
      console.error('Failed to refresh currency:', error)
    }
  }, [token])

  const refreshCurrency = refreshCurrencyInternal

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

  // M7: Plot functions
  const refreshPlotsInternal = useCallback(async () => {
    if (!token) return
    try {
      const response = await getPlots(token, { limit: 100 })
      setNearbyPlots(response.plots)
    } catch (error) {
      console.error('Failed to refresh plots:', error)
    }
  }, [token])

  const refreshPlots = refreshPlotsInternal

  // Terrain fetch (no auth required - public endpoint)
  const fetchTerrain = useCallback(async () => {
    try {
      const terrainData = await getTerrain()
      setTerrain(terrainData)
    } catch (error) {
      console.error('Failed to fetch terrain:', error)
    }
  }, [])

  const selectPlot = useCallback((plot: PlotInfo) => {
    setSelectedPlot(plot)
  }, [])

  const closePlotPanel = useCallback(() => {
    setSelectedPlot(null)
  }, [])

  const claimPlot = useCallback(async (plotId: string) => {
    if (!token) return
    try {
      const response = await claimPlotApi(token, plotId)
      setStrandBalance(response.newBalance)
      setSelectedPlot(response.plot)
      refreshPlotsInternal()
      // Notify
      const notification: Notification = {
        id: `${Date.now()}`,
        message: `Claimed plot in ${response.plot.zoneId}!`,
        timestamp: Date.now(),
      }
      setNotifications((prev) => [...prev, notification])
      setTimeout(() => {
        setNotifications((prev) => prev.filter((n) => n.id !== notification.id))
      }, 3000)
    } catch (error) {
      console.error('Failed to claim plot:', error)
      const notification: Notification = {
        id: `${Date.now()}`,
        message: error instanceof Error ? error.message : 'Failed to claim plot',
        timestamp: Date.now(),
      }
      setNotifications((prev) => [...prev, notification])
      setTimeout(() => {
        setNotifications((prev) => prev.filter((n) => n.id !== notification.id))
      }, 3000)
    }
  }, [token, refreshPlotsInternal])

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

  // M5: Trading actions
  const initiateTrade = useCallback((targetPlayerId: string) => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'trade_initiate',
        target_player_id: targetPlayerId,
      }))
    }
  }, [])

  const acceptTradeRequest = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'trade_accept_request' }))
    }
    setTradeRequest(null)
  }, [])

  const declineTradeRequest = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'trade_decline_request' }))
    }
    setTradeRequest(null)
  }, [])

  const updateTradeOffer = useCallback((items: { itemId: string; quantity: number }[], strands: number) => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: 'trade_offer',
        items: items.map(i => ({ item_id: i.itemId, quantity: i.quantity })),
        strands,
      }))
    }
  }, [])

  const acceptTrade = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'trade_accept' }))
    }
  }, [])

  const cancelTrade = useCallback(() => {
    const socket = socketRef.current
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'trade_cancel' }))
    }
    setActiveTrade(null)
  }, [])

  // Fetch terrain on mount (no auth required)
  useEffect(() => {
    fetchTerrain()
  }, [fetchTerrain])

  // Load inventory, nearby nodes, recipes, containers, stations, plots, and currency when connected
  useEffect(() => {
    if (isConnected && token) {
      refreshInventory()
      refreshNearbyNodes()
      refreshRecipes()
      refreshContainers()
      refreshStations()
      refreshPlots()
      refreshCurrency()
    }
  }, [isConnected, token, refreshInventory, refreshNearbyNodes, refreshRecipes, refreshContainers, refreshStations, refreshPlots, refreshCurrency])

  // Periodically refresh nearby nodes, stations, and plots (every 5 seconds)
  useEffect(() => {
    if (!isConnected || !token) return
    const interval = setInterval(() => {
      refreshNearbyNodes()
      refreshStations()
      refreshPlots()
    }, 5000)
    return () => clearInterval(interval)
  }, [isConnected, token, refreshNearbyNodes, refreshStations, refreshPlots])

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
    // M5: Economy
    strandBalance,
    // M5: Trading
    tradeRequest,
    activeTrade,
    sendMove,
    startExtraction,
    moveToAndExtract,
    cancelExtraction,
    startCrafting,
    cancelCrafting,
    refreshInventory,
    refreshRecipes,
    refreshContainers,
    refreshCurrency,
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
    // M5: Trading actions
    initiateTrade,
    acceptTradeRequest,
    declineTradeRequest,
    updateTradeOffer,
    acceptTrade,
    cancelTrade,
    // M7: Zones and Plots
    currentZone,
    nearbyPlots,
    selectedPlot,
    selectPlot,
    closePlotPanel,
    claimPlot,
    refreshPlots,
    // Terrain
    terrain,
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

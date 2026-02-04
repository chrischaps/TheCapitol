-- Game state management
local config = require("src.config")
local terrain = require("src.terrain")

local game = {}

-- State
game.players = {}        -- Map of player_id -> {id, name, x, y}
game.nodes = {}          -- Map of node_id -> {id, resource_type, x, y, state}
game.inventory = {}      -- Array of {id, item_type, name, quality, quality_grade, quantity}
game.myPlayerId = nil
game.isConnected = false
game.terrainLoaded = false

-- Extraction state
game.extractionState = nil -- {nodeId, progress, duration}
game.pendingExtractionNodeId = nil

-- Notifications
game.notifications = {}  -- Array of {id, message, timestamp}

-- Reset all state
function game.reset()
    game.players = {}
    game.nodes = {}
    game.inventory = {}
    game.myPlayerId = nil
    game.isConnected = false
    game.extractionState = nil
    game.pendingExtractionNodeId = nil
    game.notifications = {}
    game.terrainLoaded = false
end

-- Terrain management

function game.setTerrain(data)
    terrain.setData(data)
    game.terrainLoaded = true
end

function game.isTerrainLoaded()
    return game.terrainLoaded and terrain.isLoaded()
end

-- Player management

function game.setPlayers(players)
    for _, player in ipairs(players) do
        game.players[player.id] = player
    end
end

function game.addPlayer(player)
    game.players[player.id] = player
end

function game.removePlayer(playerId)
    game.players[playerId] = nil
end

function game.getMyPlayer()
    if game.myPlayerId then
        return game.players[game.myPlayerId]
    end
    return nil
end

-- Node management

function game.setNodes(nodes)
    for _, node in ipairs(nodes) do
        game.nodes[node.id] = node
    end
end

function game.setNodeState(nodeId, state)
    if game.nodes[nodeId] then
        game.nodes[nodeId].state = state
    end
end

function game.getNode(nodeId)
    return game.nodes[nodeId]
end

-- Extraction

function game.startExtraction(nodeId, progress, duration)
    game.extractionState = {
        nodeId = nodeId,
        progress = progress or 0,
        duration = duration,
    }
    game.pendingExtractionNodeId = nil
end

function game.updateExtractionProgress(progress)
    if game.extractionState then
        game.extractionState.progress = progress
    end
end

function game.clearExtraction()
    game.extractionState = nil
end

function game.setPendingExtraction(nodeId)
    game.pendingExtractionNodeId = nodeId
end

function game.clearPendingExtraction()
    game.pendingExtractionNodeId = nil
end

function game.isExtracting()
    return game.extractionState ~= nil
end

-- Inventory

function game.setInventory(items)
    game.inventory = items or {}
end

-- Notifications

function game.addNotification(message)
    local id = tostring(love.timer.getTime())
    table.insert(game.notifications, {
        id = id,
        message = message,
        timestamp = love.timer.getTime(),
    })
    return id
end

function game.removeNotification(id)
    for i, notif in ipairs(game.notifications) do
        if notif.id == id then
            table.remove(game.notifications, i)
            return
        end
    end
end

function game.updateNotifications()
    local now = love.timer.getTime()
    local i = 1
    while i <= #game.notifications do
        if now - game.notifications[i].timestamp > 3 then
            table.remove(game.notifications, i)
        else
            i = i + 1
        end
    end
end

-- Distance calculation

function game.getDistanceToNode(nodeId)
    local player = game.getMyPlayer()
    local node = game.nodes[nodeId]
    if not player or not node then
        return math.huge
    end
    local dx = player.x - node.x
    local dy = player.y - node.y
    return math.sqrt(dx * dx + dy * dy)
end

function game.isNodeInRange(nodeId)
    return game.getDistanceToNode(nodeId) <= config.EXTRACTION_RANGE
end

-- Check if player arrived at pending extraction target
function game.checkPendingExtraction()
    if not game.pendingExtractionNodeId then
        return nil
    end

    local node = game.nodes[game.pendingExtractionNodeId]
    if not node then
        game.pendingExtractionNodeId = nil
        return nil
    end

    -- Check if node is still available
    if node.state ~= "available" then
        game.pendingExtractionNodeId = nil
        return nil
    end

    -- Check if in range
    if game.isNodeInRange(game.pendingExtractionNodeId) then
        local nodeId = game.pendingExtractionNodeId
        game.pendingExtractionNodeId = nil
        return nodeId
    end

    return nil
end

return game

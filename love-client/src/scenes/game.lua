-- Main gameplay scene
local config = require("src.config")
local scenes = require("libs.LoveLibs.core.scenes")
local scaling = require("libs.LoveLibs.graphics.scaling")
local protocol = require("src.net.protocol")
local api = require("src.net.api")
local auth = require("src.state.auth")
local game = require("src.state.game")
local ui = require("src.state.ui")
local worldRender = require("src.render.world")
local playersRender = require("src.render.players")
local effects = require("src.render.effects")
local inventory = require("src.ui.inventory")

local gameScene = {}

-- State
local client = nil
local nodeRefreshTimer = 0
local inventoryRefreshTimer = 0

function gameScene:load(wsClient)
    client = wsClient
    ui.setConnected()
    nodeRefreshTimer = 0
    inventoryRefreshTimer = 0

    -- Initialize UI components
    inventory.init()

    -- Set up WebSocket handlers
    self:setupHandlers()

    -- Initial data fetch
    self:refreshNearbyNodes()
    self:refreshInventory()
end

function gameScene:unload()
    if client then
        client:disconnect()
        client = nil
    end
    game.reset()
end

function gameScene:setupHandlers()
    if not client then return end

    client:on("onPositions", function(players)
        game.setPlayers(players)

        -- Check if we arrived at pending extraction target
        local nodeId = game.checkPendingExtraction()
        if nodeId then
            client:extract(nodeId)
        end
    end)

    client:on("onPlayerJoined", function(player)
        game.addPlayer(player)
    end)

    client:on("onPlayerLeft", function(playerId)
        game.removePlayer(playerId)
    end)

    client:on("onExtractionStarted", function(playerId, nodeId, durationTicks)
        if playerId == game.myPlayerId then
            game.startExtraction(nodeId, 0, durationTicks)
        end
    end)

    client:on("onExtractionProgress", function(playerId, progress, duration)
        if playerId == game.myPlayerId then
            game.updateExtractionProgress(progress)
        end
    end)

    client:on("onExtractionCompleted", function(playerId, nodeId, itemName, quantity, quality)
        if playerId == game.myPlayerId then
            game.clearExtraction()
            local qualityGrade = self:qualityToGrade(quality)
            game.addNotification(itemName .. " +" .. quantity .. " (Quality: " .. qualityGrade .. ")")
            -- Refresh inventory after extraction
            self:refreshInventory()
        end
    end)

    client:on("onExtractionCancelled", function(playerId)
        if playerId == game.myPlayerId then
            game.clearExtraction()
        end
    end)

    client:on("onNodeDepleted", function(nodeId)
        game.setNodeState(nodeId, "depleted")
    end)

    client:on("onNodeRegenerated", function(nodeId)
        game.setNodeState(nodeId, "available")
    end)

    client:on("onNearbyNodes", function(nodes)
        game.setNodes(nodes)
    end)

    client:on("onDisconnect", function(code, reason)
        ui.setDisconnected(reason)
        game.isConnected = false
    end)

    client:on("onError", function(err)
        ui.setDisconnected(err)
    end)
end

function gameScene:qualityToGrade(quality)
    if quality >= 90 then return "A"
    elseif quality >= 70 then return "B"
    elseif quality >= 50 then return "C"
    elseif quality >= 30 then return "D"
    else return "F"
    end
end

function gameScene:refreshNearbyNodes()
    local token = auth.getToken()
    if not token then return end

    local co = coroutine.create(function()
        local response = api.getNearby(token)
        if response.ok and response.data and response.data.nodes then
            game.setNodes(response.data.nodes)
        end
    end)
    coroutine.resume(co)
end

function gameScene:refreshInventory()
    local token = auth.getToken()
    if not token then return end

    local co = coroutine.create(function()
        local response = api.getInventory(token)
        if response.ok and response.data and response.data.items then
            game.setInventory(response.data.items)
        end
    end)
    coroutine.resume(co)
end

function gameScene:update(dt)
    -- Update WebSocket
    if client then
        client:update()
    end

    -- Update notifications
    game.updateNotifications()

    -- Periodic node refresh
    nodeRefreshTimer = nodeRefreshTimer + dt
    if nodeRefreshTimer >= config.NODE_REFRESH_INTERVAL then
        nodeRefreshTimer = 0
        self:refreshNearbyNodes()
    end

    -- Update inventory UI
    local vw, vh = scaling.getVirtualSize()
    local mx, my = scaling.toVirtual(love.mouse.getPosition())
    inventory.update(dt, vw, vh, mx, my)
end

function gameScene:keypressed(key)
    if key == "escape" then
        if client then
            client:disconnect()
        end
        auth.clear()
        scenes.switchTo("login")
        return
    end

    if inventory.keypressed(key) then
        return
    end
end

function gameScene:mousepressed(x, y, button)
    if button ~= 1 then return end

    local vw, vh = scaling.getVirtualSize()
    local vx, vy = scaling.toVirtual(x, y)

    -- Check UI first
    if inventory.mousepressed(vx, vy, button, vw, vh) then
        return
    end

    -- Handle world click
    self:handleWorldClick(vx, vy, vw, vh)
end

function gameScene:mousereleased(x, y, button)
    local vx, vy = scaling.toVirtual(x, y)
    inventory.mousereleased(vx, vy, button)
end

function gameScene:wheelmoved(x, y)
    local vw, vh = scaling.getVirtualSize()
    inventory.wheelmoved(x, y, vw, vh)
end

function gameScene:handleWorldClick(vx, vy, screenWidth, screenHeight)
    if not client or not game.myPlayerId then return end

    local player = game.getMyPlayer()
    if not player then return end

    -- Calculate camera offset
    local offsetX = screenWidth / 2 - player.x
    local offsetY = screenHeight / 2 - player.y

    -- Convert screen coords to world coords
    local worldX = vx - offsetX
    local worldY = vy - offsetY

    -- Check if clicking on a resource node
    local clickedNode = nil
    local nodeRadius = config.NODE_RADIUS

    for id, node in pairs(game.nodes) do
        local dx = worldX - node.x
        local dy = worldY - node.y
        local distance = math.sqrt(dx * dx + dy * dy)
        if distance <= nodeRadius + 4 then
            clickedNode = node
            break
        end
    end

    if clickedNode then
        -- Only interact with available nodes
        if clickedNode.state == "available" then
            -- Check if in range
            local distanceToNode = game.getDistanceToNode(clickedNode.id)

            if distanceToNode <= config.EXTRACTION_RANGE then
                -- Start extraction immediately
                client:extract(clickedNode.id)
            else
                -- Move to node and extract when in range
                game.setPendingExtraction(clickedNode.id)
                client:move(clickedNode.x, clickedNode.y)
            end
            return
        end
    end

    -- Regular movement
    local clampedX = math.max(0, math.min(config.WORLD_SIZE, worldX))
    local clampedY = math.max(0, math.min(config.WORLD_SIZE, worldY))

    -- Clear pending extraction when manually moving
    game.clearPendingExtraction()
    client:move(clampedX, clampedY)
end

function gameScene:draw()
    local colors = config.COLORS
    local vw, vh = scaling.getVirtualSize()

    -- Background
    love.graphics.setColor(colors.BACKGROUND)
    love.graphics.rectangle("fill", 0, 0, vw, vh)

    -- Calculate camera offset
    local player = game.getMyPlayer()
    local cameraX = player and player.x or config.WORLD_SIZE / 2
    local cameraY = player and player.y or config.WORLD_SIZE / 2
    local offsetX = vw / 2 - cameraX
    local offsetY = vh / 2 - cameraY

    -- Draw world
    worldRender.drawGrid(offsetX, offsetY, vw, vh)
    worldRender.drawBoundary(offsetX, offsetY)
    worldRender.drawNodes(offsetX, offsetY)
    worldRender.drawTargetLine(offsetX, offsetY)

    -- Draw players
    playersRender.draw(offsetX, offsetY)

    -- Draw extraction ring
    effects.drawExtractionRing(offsetX, offsetY)

    -- Draw UI
    inventory.draw(vw, vh)
    effects.drawNotifications(vw, vh)

    -- Connection status overlay
    if not game.isConnected then
        effects.drawConnectionStatus(vw, vh, ui.connectionStatus)
    end
end

return gameScene

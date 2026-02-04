-- WebSocket client wrapper for The Capitol
local websocket = require("libs.websocket")
local json = require("libs.json")
local config = require("src.config")
local protocol = require("src.net.protocol")

local ws = {}
ws.__index = ws

function ws.new()
    local self = setmetatable({}, ws)
    self.client = nil
    self.isConnected = false
    self.isAuthenticated = false
    self.playerId = nil
    self.messageQueue = {}
    self.handlers = {}
    self.lastError = nil
    return self
end

function ws:connect()
    if self.client then
        self:disconnect()
    end

    print("[WS] Connecting to " .. config.WS_HOST .. ":" .. config.WS_PORT .. config.WS_PATH)

    local ok, client_or_err = pcall(websocket.new, config.WS_HOST, config.WS_PORT, config.WS_PATH)
    if not ok then
        print("[WS] Failed to create websocket: " .. tostring(client_or_err))
        self.lastError = "Failed to create socket: " .. tostring(client_or_err)
        if self.handlers.onError then
            self.handlers.onError(self.lastError)
        end
        return
    end

    self.client = client_or_err
    self.isConnected = false
    self.isAuthenticated = false

    -- Check if connection failed during construction
    if self.client.status == websocket.STATUS.CLOSED then
        local err = self.client._connectError or "connection failed"
        print("[WS] Connection failed during setup: " .. tostring(err))
        self.lastError = err
        if self.handlers.onError then
            self.handlers.onError(err)
        end
        return
    end

    -- Set up callbacks
    local self_ref = self
    function self.client:onopen()
        print("[WS] Connection opened!")
        self_ref.isConnected = true
        if self_ref.handlers.onConnect then
            self_ref.handlers.onConnect()
        end
    end

    function self.client:onmessage(message)
        print("[WS] Received: " .. message:sub(1, 100))
        local ok, msg = pcall(json.decode, message)
        if ok and msg then
            self_ref:handleMessage(msg)
        else
            print("[WS] Failed to parse message: " .. tostring(msg))
        end
    end

    function self.client:onerror(err)
        print("[WS] Error: " .. tostring(err))
        self_ref.lastError = err
        if self_ref.handlers.onError then
            self_ref.handlers.onError(err)
        end
    end

    function self.client:onclose(code, reason)
        print("[WS] Connection closed: " .. tostring(code) .. " - " .. tostring(reason))
        self_ref.isConnected = false
        self_ref.isAuthenticated = false
        if self_ref.handlers.onDisconnect then
            self_ref.handlers.onDisconnect(code, reason)
        end
    end
end

function ws:disconnect()
    if self.client then
        self.client:close()
        self.client = nil
    end
    self.isConnected = false
    self.isAuthenticated = false
end

function ws:update()
    if self.client then
        -- Debug: print status periodically
        if self.client.status and self.client._length then
            if self.client._length % 60 == 0 then  -- Every ~1 second
                print("[WS] Status: " .. self.client.status .. ", attempts: " .. self.client._length)
            end
        end
        self.client:update()
    end
end

function ws:send(data)
    if self.client and self.isConnected then
        local message = json.encode(data)
        self.client:send(message)
    end
end

-- Message handlers

function ws:handleMessage(msg)
    local msgType = msg.type

    if msgType == protocol.SERVER.AUTH_OK then
        self.isAuthenticated = true
        self.playerId = msg.player_id
        if self.handlers.onAuthOk then
            self.handlers.onAuthOk(msg.player_id)
        end

    elseif msgType == protocol.SERVER.AUTH_ERROR then
        if self.handlers.onAuthError then
            self.handlers.onAuthError(msg.message)
        end

    elseif msgType == protocol.SERVER.SUBSCRIBED then
        if self.handlers.onSubscribed then
            self.handlers.onSubscribed(msg.channel)
        end

    elseif msgType == protocol.SERVER.POSITIONS then
        if self.handlers.onPositions then
            self.handlers.onPositions(msg.players)
        end

    elseif msgType == protocol.SERVER.PLAYER_JOINED then
        if self.handlers.onPlayerJoined then
            self.handlers.onPlayerJoined(msg.player)
        end

    elseif msgType == protocol.SERVER.PLAYER_LEFT then
        if self.handlers.onPlayerLeft then
            self.handlers.onPlayerLeft(msg.player_id)
        end

    elseif msgType == protocol.SERVER.EXTRACTION_STARTED then
        if self.handlers.onExtractionStarted then
            self.handlers.onExtractionStarted(msg.player_id, msg.node_id, msg.duration_ticks)
        end

    elseif msgType == protocol.SERVER.EXTRACTION_PROGRESS then
        if self.handlers.onExtractionProgress then
            self.handlers.onExtractionProgress(msg.player_id, msg.progress, msg.duration)
        end

    elseif msgType == protocol.SERVER.EXTRACTION_COMPLETED then
        if self.handlers.onExtractionCompleted then
            self.handlers.onExtractionCompleted(msg.player_id, msg.node_id, msg.item_name, msg.quantity, msg.quality)
        end

    elseif msgType == protocol.SERVER.EXTRACTION_CANCELLED then
        if self.handlers.onExtractionCancelled then
            self.handlers.onExtractionCancelled(msg.player_id)
        end

    elseif msgType == protocol.SERVER.NODE_DEPLETED then
        if self.handlers.onNodeDepleted then
            self.handlers.onNodeDepleted(msg.node_id)
        end

    elseif msgType == protocol.SERVER.NODE_REGENERATED then
        if self.handlers.onNodeRegenerated then
            self.handlers.onNodeRegenerated(msg.node_id)
        end

    elseif msgType == protocol.SERVER.NEARBY_NODES then
        if self.handlers.onNearbyNodes then
            self.handlers.onNearbyNodes(msg.nodes)
        end

    elseif msgType == protocol.SERVER.ERROR then
        if self.handlers.onError then
            self.handlers.onError(msg.message)
        end

    elseif msgType == protocol.SERVER.MOVEMENT_BLOCKED then
        if self.handlers.onMovementBlocked then
            self.handlers.onMovementBlocked(msg.player_id, msg.reason, msg.stopped_x, msg.stopped_y)
        end

    elseif msgType == protocol.SERVER.ZONE_CHANGED then
        if self.handlers.onZoneChanged then
            self.handlers.onZoneChanged(msg.player_id, msg.from_zone, msg.to_zone, msg.zone_name)
        end
    end
end

-- Outgoing messages

function ws:auth(token)
    self:send({
        type = protocol.CLIENT.AUTH,
        token = token,
    })
end

function ws:subscribe(channel)
    self:send({
        type = protocol.CLIENT.SUBSCRIBE,
        channel = channel,
    })
end

function ws:move(x, y)
    self:send({
        type = protocol.CLIENT.MOVE,
        x = x,
        y = y,
    })
end

function ws:extract(nodeId)
    self:send({
        type = protocol.CLIENT.EXTRACT,
        node_id = nodeId,
    })
end

function ws:cancelExtraction()
    self:send({
        type = protocol.CLIENT.CANCEL_EXTRACTION,
    })
end

-- Handler registration

function ws:on(event, handler)
    self.handlers[event] = handler
end

return ws

-- Loading/connection scene
local config = require("src.config")
local scenes = require("libs.LoveLibs.core.scenes")
local scaling = require("libs.LoveLibs.graphics.scaling")
local protocol = require("src.net.protocol")
local ws = require("src.net.ws")
local auth = require("src.state.auth")
local game = require("src.state.game")

local loading = {}

-- State
local client = nil
local statusMessage = "Connecting..."
local errorMessage = nil
local connectionAttempts = 0
local maxAttempts = 3

function loading:load()
    statusMessage = "Connecting..."
    errorMessage = nil
    connectionAttempts = 0
    game.reset()

    self:connectWebSocket()
end

function loading:unload()
    -- Don't disconnect here - the game scene will take over the connection
end

function loading:connectWebSocket()
    connectionAttempts = connectionAttempts + 1

    if client then
        client:disconnect()
    end

    client = ws.new()

    -- Set up handlers
    client:on("onConnect", function()
        statusMessage = "Authenticating..."
        local token = auth.getToken()
        if token then
            client:auth(token)
        else
            errorMessage = "No authentication token"
            statusMessage = "Error"
        end
    end)

    client:on("onAuthOk", function(playerId)
        statusMessage = "Authenticated! Subscribing..."
        game.myPlayerId = playerId
        auth.setAuth(auth.getToken(), playerId)
        client:subscribe(protocol.CHANNELS.POSITION)
    end)

    client:on("onAuthError", function(message)
        errorMessage = message or "Authentication failed"
        statusMessage = "Error"
        auth.clear()
    end)

    client:on("onSubscribed", function(channel)
        if channel == protocol.CHANNELS.POSITION then
            statusMessage = "Connected!"
            game.isConnected = true
            -- Pass the client to the game scene
            scenes.switchTo("game", client)
        end
    end)

    client:on("onError", function(err)
        if connectionAttempts < maxAttempts then
            statusMessage = "Connection failed, retrying..."
            self:connectWebSocket()
        else
            errorMessage = err or "Connection failed"
            statusMessage = "Error"
        end
    end)

    client:on("onDisconnect", function(code, reason)
        if not game.isConnected then
            if connectionAttempts < maxAttempts then
                statusMessage = "Connection lost, retrying..."
                self:connectWebSocket()
            else
                errorMessage = "Could not connect to server"
                statusMessage = "Error"
            end
        end
    end)

    client:connect()
end

function loading:update(dt)
    if client then
        client:update()
    end
end

function loading:keypressed(key)
    if key == "escape" then
        if client then
            client:disconnect()
        end
        scenes.switchTo("login")
    end

    if key == "return" and errorMessage then
        -- Retry connection
        connectionAttempts = 0
        errorMessage = nil
        self:connectWebSocket()
    end
end

function loading:draw()
    local colors = config.COLORS
    local vw, vh = scaling.getVirtualSize()

    -- Background
    love.graphics.setColor(colors.BACKGROUND)
    love.graphics.rectangle("fill", 0, 0, vw, vh)

    -- Status message
    love.graphics.setColor(colors.GOLDEN)
    local font = love.graphics.getFont()
    local statusWidth = font:getWidth(statusMessage)
    love.graphics.print(statusMessage, (vw - statusWidth) / 2, vh / 2 - 20)

    -- Error message
    if errorMessage then
        love.graphics.setColor(colors.QUALITY_F)
        local errorWidth = font:getWidth(errorMessage)
        love.graphics.print(errorMessage, (vw - errorWidth) / 2, vh / 2 + 10)

        love.graphics.setColor(colors.TEXT_DIM)
        local hint = "Press ENTER to retry or ESC to go back"
        local hintWidth = font:getWidth(hint)
        love.graphics.print(hint, (vw - hintWidth) / 2, vh / 2 + 40)
    end

    -- Loading dots animation
    if not errorMessage then
        local dots = string.rep(".", math.floor(love.timer.getTime() * 2) % 4)
        love.graphics.setColor(colors.TEXT_DIM)
        love.graphics.print(dots, (vw + statusWidth) / 2 + 5, vh / 2 - 20)
    end
end

return loading

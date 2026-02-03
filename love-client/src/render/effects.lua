-- Visual effects rendering (extraction ring, etc.)
local config = require("src.config")
local game = require("src.state.game")

local effects = {}

function effects.drawExtractionRing(offsetX, offsetY)
    if not game.extractionState then
        return
    end

    local player = game.getMyPlayer()
    if not player then
        return
    end

    local colors = config.COLORS
    local playerRadius = config.PLAYER_RADIUS

    local screenX = player.x + offsetX
    local screenY = player.y + offsetY

    local progress = game.extractionState.progress / game.extractionState.duration
    local ringRadius = playerRadius + 6

    -- Background ring
    love.graphics.setColor(colors.EXTRACTION_RING_BG)
    love.graphics.setLineWidth(3)
    love.graphics.circle("line", screenX, screenY, ringRadius)

    -- Progress arc
    local startAngle = -math.pi / 2
    local endAngle = startAngle + progress * math.pi * 2

    love.graphics.setColor(colors.EXTRACTION_RING)
    love.graphics.setLineWidth(3)
    love.graphics.arc("line", "open", screenX, screenY, ringRadius, startAngle, endAngle, 32)
end

function effects.drawNotifications(screenWidth, screenHeight)
    if #game.notifications == 0 then
        return
    end

    local colors = config.COLORS
    local font = love.graphics.getFont()

    love.graphics.setColor(colors.GOLDEN)
    for i, notif in ipairs(game.notifications) do
        local y = 30 + (i - 1) * 24
        local textWidth = font:getWidth(notif.message)
        love.graphics.print(notif.message, screenWidth - textWidth - 20, y)
    end
end

function effects.drawConnectionStatus(screenWidth, screenHeight, status)
    if status == "connected" then
        return
    end

    local colors = config.COLORS

    -- Overlay
    love.graphics.setColor(0, 0, 0, 0.7)
    love.graphics.rectangle("fill", 0, 0, screenWidth, screenHeight)

    -- Status text
    love.graphics.setColor(colors.GOLDEN)
    local message = status == "connecting" and "Connecting..." or "Disconnected"
    local font = love.graphics.getFont()
    local textWidth = font:getWidth(message)
    love.graphics.print(message, (screenWidth - textWidth) / 2, screenHeight / 2)
end

return effects

-- Player rendering
local config = require("src.config")
local game = require("src.state.game")

local players = {}

function players.draw(offsetX, offsetY)
    local colors = config.COLORS
    local radius = config.PLAYER_RADIUS

    for id, player in pairs(game.players) do
        local screenX = player.x + offsetX
        local screenY = player.y + offsetY
        local isMe = id == game.myPlayerId

        -- Player circle
        if isMe then
            love.graphics.setColor(colors.PLAYER_SELF)
        else
            love.graphics.setColor(colors.PLAYER_OTHER)
        end
        love.graphics.circle("fill", screenX, screenY, radius)

        -- Player stroke
        if isMe then
            love.graphics.setColor(colors.PLAYER_SELF_STROKE)
        else
            love.graphics.setColor(colors.PLAYER_OTHER_STROKE)
        end
        love.graphics.setLineWidth(2)
        love.graphics.circle("line", screenX, screenY, radius)

        -- Player name
        love.graphics.setColor(colors.TEXT)
        local font = love.graphics.getFont()
        local nameWidth = font:getWidth(player.name)
        love.graphics.print(player.name, screenX - nameWidth / 2, screenY - radius - 18)
    end
end

return players

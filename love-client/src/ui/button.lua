-- Reusable button component
local config = require("src.config")

local button = {}
button.__index = button

function button.new(x, y, width, height, text, onClick)
    local self = setmetatable({}, button)
    self.x = x
    self.y = y
    self.width = width
    self.height = height
    self.text = text
    self.onClick = onClick
    self.isHovered = false
    self.isPressed = false
    self.enabled = true
    return self
end

function button:setPosition(x, y)
    self.x = x
    self.y = y
end

function button:contains(px, py)
    return px >= self.x and px <= self.x + self.width
       and py >= self.y and py <= self.y + self.height
end

function button:update(mx, my)
    self.isHovered = self:contains(mx, my)
end

function button:mousepressed(x, y, btn)
    if btn == 1 and self.enabled and self:contains(x, y) then
        self.isPressed = true
        return true
    end
    return false
end

function button:mousereleased(x, y, btn)
    if btn == 1 and self.isPressed then
        self.isPressed = false
        if self.enabled and self:contains(x, y) and self.onClick then
            self.onClick()
            return true
        end
    end
    return false
end

function button:draw()
    local colors = config.COLORS

    -- Background
    if not self.enabled then
        love.graphics.setColor(colors.PANEL_BORDER)
    elseif self.isPressed then
        love.graphics.setColor(colors.GOLDEN[1] * 0.7, colors.GOLDEN[2] * 0.7, colors.GOLDEN[3] * 0.7, 1)
    elseif self.isHovered then
        love.graphics.setColor(colors.GOLDEN)
    else
        love.graphics.setColor(colors.PANEL_BG)
    end
    love.graphics.rectangle("fill", self.x, self.y, self.width, self.height, 4, 4)

    -- Border
    if self.enabled then
        love.graphics.setColor(colors.GOLDEN)
    else
        love.graphics.setColor(colors.TEXT_DIM)
    end
    love.graphics.setLineWidth(2)
    love.graphics.rectangle("line", self.x, self.y, self.width, self.height, 4, 4)

    -- Text
    if self.isHovered and self.enabled and not self.isPressed then
        love.graphics.setColor(colors.BACKGROUND)
    elseif self.enabled then
        love.graphics.setColor(colors.TEXT)
    else
        love.graphics.setColor(colors.TEXT_DIM)
    end

    local font = love.graphics.getFont()
    local textWidth = font:getWidth(self.text)
    local textHeight = font:getHeight()
    love.graphics.print(
        self.text,
        self.x + (self.width - textWidth) / 2,
        self.y + (self.height - textHeight) / 2
    )
end

return button

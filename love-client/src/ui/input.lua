-- Text input field component
local config = require("src.config")

local input = {}
input.__index = input

function input.new(x, y, width, height, placeholder)
    local self = setmetatable({}, input)
    self.x = x
    self.y = y
    self.width = width
    self.height = height
    self.placeholder = placeholder or ""
    self.text = ""
    self.isFocused = false
    self.cursorPos = 0
    self.cursorBlink = 0
    self.isPassword = false
    self.maxLength = 100
    return self
end

function input:setPosition(x, y)
    self.x = x
    self.y = y
end

function input:contains(px, py)
    return px >= self.x and px <= self.x + self.width
       and py >= self.y and py <= self.y + self.height
end

function input:focus()
    self.isFocused = true
    self.cursorPos = #self.text
    self.cursorBlink = 0
end

function input:unfocus()
    self.isFocused = false
end

function input:getValue()
    return self.text
end

function input:setValue(text)
    self.text = text or ""
    self.cursorPos = #self.text
end

function input:clear()
    self.text = ""
    self.cursorPos = 0
end

function input:update(dt)
    if self.isFocused then
        self.cursorBlink = self.cursorBlink + dt
        if self.cursorBlink > 1 then
            self.cursorBlink = 0
        end
    end
end

function input:mousepressed(x, y, btn)
    if btn == 1 then
        if self:contains(x, y) then
            self:focus()
            return true
        else
            self:unfocus()
        end
    end
    return false
end

function input:keypressed(key)
    if not self.isFocused then
        return false
    end

    if key == "backspace" then
        if self.cursorPos > 0 then
            self.text = self.text:sub(1, self.cursorPos - 1) .. self.text:sub(self.cursorPos + 1)
            self.cursorPos = self.cursorPos - 1
        end
        return true
    elseif key == "delete" then
        if self.cursorPos < #self.text then
            self.text = self.text:sub(1, self.cursorPos) .. self.text:sub(self.cursorPos + 2)
        end
        return true
    elseif key == "left" then
        self.cursorPos = math.max(0, self.cursorPos - 1)
        return true
    elseif key == "right" then
        self.cursorPos = math.min(#self.text, self.cursorPos + 1)
        return true
    elseif key == "home" then
        self.cursorPos = 0
        return true
    elseif key == "end" then
        self.cursorPos = #self.text
        return true
    end

    return false
end

function input:textinput(t)
    if not self.isFocused then
        return false
    end

    if #self.text < self.maxLength then
        self.text = self.text:sub(1, self.cursorPos) .. t .. self.text:sub(self.cursorPos + 1)
        self.cursorPos = self.cursorPos + #t
    end
    return true
end

function input:draw()
    local colors = config.COLORS

    -- Background
    love.graphics.setColor(colors.INPUT_BG)
    love.graphics.rectangle("fill", self.x, self.y, self.width, self.height, 4, 4)

    -- Border
    if self.isFocused then
        love.graphics.setColor(colors.GOLDEN)
    else
        love.graphics.setColor(colors.PANEL_BORDER)
    end
    love.graphics.setLineWidth(2)
    love.graphics.rectangle("line", self.x, self.y, self.width, self.height, 4, 4)

    -- Text or placeholder
    local padding = 10
    local font = love.graphics.getFont()
    local textHeight = font:getHeight()
    local textY = self.y + (self.height - textHeight) / 2

    local displayText = self.text
    if self.isPassword then
        displayText = string.rep("*", #self.text)
    end

    love.graphics.setScissor(self.x + padding, self.y, self.width - padding * 2, self.height)

    if displayText == "" and not self.isFocused then
        love.graphics.setColor(colors.TEXT_DIM)
        love.graphics.print(self.placeholder, self.x + padding, textY)
    else
        love.graphics.setColor(colors.TEXT)
        love.graphics.print(displayText, self.x + padding, textY)
    end

    -- Cursor
    if self.isFocused and self.cursorBlink < 0.5 then
        local cursorDisplay = displayText:sub(1, self.cursorPos)
        local cursorX = self.x + padding + font:getWidth(cursorDisplay)
        love.graphics.setColor(colors.GOLDEN)
        love.graphics.rectangle("fill", cursorX, textY, 2, textHeight)
    end

    love.graphics.setScissor()
end

return input

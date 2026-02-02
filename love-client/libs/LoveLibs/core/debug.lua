--[[
    love2d-debug
    Debug overlay system for LOVE 2D

    Provides a toggleable debug panel that displays FPS, custom metrics,
    and game-specific debug information. Games can expose custom debug
    data through a simple interface.

    USAGE:
        local debug = require("libs.debug")

        function love.keypressed(key)
            if key == "f3" then
                debug.toggle()
            end
        end

        function love.draw()
            -- Your game drawing...

            -- Draw debug overlay last (on top)
            debug.draw(currentEntity)  -- Pass entity with getDebugInfo()
        end

    CUSTOM DEBUG INFO:
        To expose custom debug info, implement getDebugInfo() on your game object:

        function MyGame:getDebugInfo()
            return {
                { section = "Player" },                     -- Section header
                { key = "Health", value = self.health },    -- Key-value pair
                { key = "Position", value = self.x .. ", " .. self.y },
                { bar = "Stamina", value = self.stamina / 100, color = {0.2, 0.8, 0.2} },  -- Progress bar
                { section = "World" },
                { key = "Enemies", value = #self.enemies },
            }
        end

    API:
        debug.toggle()
            Toggle debug overlay visibility

        debug.show()
            Show debug overlay

        debug.hide()
            Hide debug overlay

        debug.isEnabled() -> boolean
            Check if debug overlay is visible

        debug.draw(target)
            Draw the debug overlay
            target: Optional object with getDebugInfo() method

        debug.setPosition(x, y)
            Set overlay position

        debug.setSize(width, height)
            Set overlay dimensions

        debug.setFont(font)
            Set custom font for debug text

        debug.setColors(config)
            Set overlay colors
            config = { background, text, header, value, section }

        debug.addGlobalInfo(key, valueFunc)
            Add persistent global info that always shows
            valueFunc: function that returns the current value

        debug.removeGlobalInfo(key)
            Remove global info

    LICENSE: MIT
]]

local debug = {}

-- Internal state
local enabled = false
local font = nil
local customFont = false

-- Position and size
local config = {
    x = 10,
    y = 50,
    width = 220,
    height = 400,
    padding = 5,
    lineHeight = 14,
    sectionSpacing = 6,
    barHeight = 12,
    barWidth = 180
}

-- Colors
local colors = {
    background = {0, 0, 0, 0.75},
    text = {0.7, 0.7, 0.7, 0.9},
    header = {1, 1, 0, 0.9},
    value = {0.6, 0.8, 0.6, 0.9},
    section = {0.8, 0.8, 0.5, 0.9},
    barBg = {0.4, 0.4, 0.4, 0.9},
    barFill = {0.4, 0.8, 0.4, 0.9}
}

-- Global info that always shows
local globalInfo = {}

--------------------------------------------------
-- PUBLIC API
--------------------------------------------------

--- Toggle debug overlay visibility
function debug.toggle()
    enabled = not enabled
end

--- Show debug overlay
function debug.show()
    enabled = true
end

--- Hide debug overlay
function debug.hide()
    enabled = false
end

--- Check if debug overlay is visible
-- @return boolean
function debug.isEnabled()
    return enabled
end

--- Set overlay position
-- @param x X position
-- @param y Y position
function debug.setPosition(x, y)
    config.x = x
    config.y = y
end

--- Set overlay dimensions
-- @param width Panel width
-- @param height Panel height (or nil for auto)
function debug.setSize(width, height)
    config.width = width or config.width
    config.height = height  -- nil = auto height
end

--- Set custom font for debug text
-- @param newFont LOVE font object
function debug.setFont(newFont)
    font = newFont
    customFont = true
end

--- Set overlay colors
-- @param newColors Table with color keys
function debug.setColors(newColors)
    for k, v in pairs(newColors) do
        colors[k] = v
    end
end

--- Add persistent global info
-- @param key Display key name
-- @param valueFunc Function that returns the current value
function debug.addGlobalInfo(key, valueFunc)
    globalInfo[key] = valueFunc
end

--- Remove global info
-- @param key Key name to remove
function debug.removeGlobalInfo(key)
    globalInfo[key] = nil
end

--- Clear all global info
function debug.clearGlobalInfo()
    globalInfo = {}
end

--- Draw the debug overlay
-- @param target Optional object with getDebugInfo() method
function debug.draw(target)
    if not enabled then return end

    -- Lazy init font
    if not font then
        font = love.graphics.newFont(12)
    end

    local oldFont = love.graphics.getFont()
    love.graphics.setFont(font)

    -- Background panel
    love.graphics.setColor(colors.background)
    love.graphics.rectangle("fill", config.x, config.y, config.width, config.height)

    love.graphics.setColor(colors.header)
    love.graphics.print("DEBUG (F3 to toggle)", config.x + config.padding, config.y + config.padding)

    local y = config.y + config.padding + 20

    -- FPS (always shown)
    love.graphics.setColor(colors.text)
    love.graphics.print(string.format("FPS: %d", love.timer.getFPS()), config.x + config.padding, y)
    y = y + config.lineHeight

    -- Memory usage
    local memKB = collectgarbage("count")
    love.graphics.print(string.format("Memory: %.1f MB", memKB / 1024), config.x + config.padding, y)
    y = y + config.lineHeight

    -- Global info
    for key, valueFunc in pairs(globalInfo) do
        love.graphics.setColor(colors.value)
        local value = valueFunc()
        local valueStr = type(value) == "number" and string.format("%.3f", value) or tostring(value)
        love.graphics.print(string.format("%s: %s", key, valueStr), config.x + config.padding, y)
        y = y + config.lineHeight
    end

    -- Target-specific debug info
    if target and target.getDebugInfo then
        local info = target:getDebugInfo()

        y = y + config.sectionSpacing
        love.graphics.setColor(colors.header)
        love.graphics.print("-- Custom --", config.x + config.padding, y)
        y = y + config.lineHeight + 2

        for _, item in ipairs(info) do
            if item.section then
                -- Section header
                y = y + config.sectionSpacing
                love.graphics.setColor(colors.section)
                love.graphics.print(item.section, config.x + config.padding, y)
                y = y + config.lineHeight + 2

            elseif item.key then
                -- Key-value pair
                love.graphics.setColor(colors.value)
                local valueStr = tostring(item.value)
                if type(item.value) == "number" then
                    valueStr = string.format("%.3f", item.value)
                end
                love.graphics.print(string.format("%s: %s", item.key, valueStr), config.x + config.padding + 5, y)
                y = y + config.lineHeight

            elseif item.bar then
                -- Progress bar visualization
                local barX = config.x + config.padding + 5
                local barValue = math.max(0, math.min(1, item.value or 0))

                love.graphics.setColor(colors.barBg)
                love.graphics.rectangle("fill", barX, y, config.barWidth, config.barHeight)

                local barColor = item.color or colors.barFill
                if type(barColor) == "table" and #barColor >= 3 then
                    love.graphics.setColor(barColor[1], barColor[2], barColor[3], barColor[4] or 0.9)
                else
                    love.graphics.setColor(colors.barFill)
                end
                love.graphics.rectangle("fill", barX, y, config.barWidth * barValue, config.barHeight)

                love.graphics.setColor(1, 1, 1, 0.9)
                love.graphics.print(item.bar, barX + 5, y)
                y = y + config.barHeight + 4
            end
        end
    else
        y = y + config.sectionSpacing
        love.graphics.setColor(0.5, 0.5, 0.5, 0.9)
        love.graphics.print("No debug info available", config.x + config.padding, y)
        love.graphics.print("(implement getDebugInfo())", config.x + config.padding, y + 14)
    end

    love.graphics.setFont(oldFont)
end

--------------------------------------------------
-- UTILITY FUNCTIONS
--------------------------------------------------

--- Draw a simple value at a position (for quick debugging)
-- @param label Label text
-- @param value Value to display
-- @param x X position
-- @param y Y position
function debug.drawValue(label, value, x, y)
    if not enabled then return end

    love.graphics.setColor(colors.value)
    local valueStr = type(value) == "number" and string.format("%.2f", value) or tostring(value)
    love.graphics.print(string.format("%s: %s", label, valueStr), x, y)
end

--- Draw a point marker for debugging positions
-- @param x X position
-- @param y Y position
-- @param color Optional color (default red)
-- @param size Optional size (default 5)
function debug.drawPoint(x, y, color, size)
    if not enabled then return end

    color = color or {1, 0, 0, 1}
    size = size or 5

    love.graphics.setColor(color)
    love.graphics.circle("fill", x, y, size)
    love.graphics.setColor(1, 1, 1, 0.8)
    love.graphics.print(string.format("%.0f, %.0f", x, y), x + size + 2, y - 6)
end

--- Draw a rectangle outline for debugging bounds
-- @param x X position
-- @param y Y position
-- @param w Width
-- @param h Height
-- @param color Optional color (default green)
function debug.drawRect(x, y, w, h, color)
    if not enabled then return end

    color = color or {0, 1, 0, 0.8}
    love.graphics.setColor(color)
    love.graphics.rectangle("line", x, y, w, h)
end

--- Draw a line for debugging vectors/directions
-- @param x1 Start X
-- @param y1 Start Y
-- @param x2 End X
-- @param y2 End Y
-- @param color Optional color (default blue)
function debug.drawLine(x1, y1, x2, y2, color)
    if not enabled then return end

    color = color or {0, 0.5, 1, 0.8}
    love.graphics.setColor(color)
    love.graphics.line(x1, y1, x2, y2)
end

return debug

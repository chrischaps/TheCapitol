--[[
    love2d-scaling
    Resolution-independent scaling system for LOVE 2D

    Maintains a virtual resolution while scaling to fit any window size,
    with automatic letterboxing to preserve aspect ratio.

    Merged from Emily and Fishbowl projects with combined features.

    USAGE:
        local scaling = require("LoveLibs.graphics.scaling")

        function love.load()
            scaling.init(960, 540)  -- Set your design resolution
        end

        function love.resize(w, h)
            scaling.resize(w, h)
        end

        function love.draw()
            scaling.start()
            -- All your drawing code here uses virtual coordinates
            love.graphics.circle("fill", 480, 270, 50)
            scaling.stop()
        end

        function love.mousepressed(x, y, button)
            local vx, vy = scaling.toVirtual(x, y)
            -- vx, vy are in virtual coordinates
        end

    API:
        scaling.init(virtualWidth, virtualHeight)
            Initialize with your design resolution

        scaling.resize(windowWidth, windowHeight)
            Call from love.resize to recalculate scaling (or call update())

        scaling.update()
            Recalculate scaling based on current window size

        scaling.start()
            Begin scaled drawing (call at start of love.draw)

        scaling.stop()
            End scaled drawing (call at end of love.draw)

        scaling.apply() / scaling.reset()
            Aliases for start()/stop() for compatibility

        scaling.toVirtual(screenX, screenY) -> virtualX, virtualY
            Convert screen coordinates to virtual coordinates (for mouse input)

        scaling.toGame(screenX, screenY) -> virtualX, virtualY
            Alias for toVirtual() for compatibility

        scaling.toScreen(virtualX, virtualY) -> screenX, screenY
            Convert virtual coordinates to screen coordinates

        scaling.isInBounds(screenX, screenY) -> boolean
            Check if screen coordinates are within the game area

        scaling.getScale() -> number
            Get current scale factor

        scaling.getOffset() -> offsetX, offsetY
            Get letterbox offset

        scaling.getTransform() -> scale, offsetX, offsetY
            Get all transform values at once

        scaling.getVirtualSize() -> width, height
            Get virtual resolution

        scaling.getNative() -> width, height
            Alias for getVirtualSize() for compatibility

        scaling.getActualSize() -> width, height
            Get actual window size

        scaling.getLetterbox() -> {left, right, top, bottom}
            Get letterbox/pillarbox dimensions

        scaling.drawBars(r, g, b)
            Draw letterbox/pillarbox bars with custom color

        scaling.setScissor()
            Set scissor to clip to game area

        scaling.clearScissor()
            Clear scissor

    LICENSE: MIT
]]

local scaling = {}

-- Virtual (design) resolution
local virtualWidth = 960
local virtualHeight = 540

-- Calculated values
local scale = 1
local offsetX = 0
local offsetY = 0
local actualWidth = virtualWidth
local actualHeight = virtualHeight

--- Initialize the scaling system with a virtual resolution
-- @param width Virtual width (design resolution)
-- @param height Virtual height (design resolution)
function scaling.init(width, height)
    if width then virtualWidth = width end
    if height then virtualHeight = height end

    local w, h = love.graphics.getDimensions()
    scaling.resize(w, h)
end

--- Recalculate scaling for a new window size
-- @param windowWidth New window width
-- @param windowHeight New window height
function scaling.resize(windowWidth, windowHeight)
    actualWidth = windowWidth
    actualHeight = windowHeight

    -- Calculate scale to fit while maintaining aspect ratio
    local scaleX = windowWidth / virtualWidth
    local scaleY = windowHeight / virtualHeight

    -- Use the smaller scale to ensure everything fits (letterboxing)
    scale = math.min(scaleX, scaleY)

    -- Calculate offset to center the game
    local scaledWidth = virtualWidth * scale
    local scaledHeight = virtualHeight * scale
    offsetX = (windowWidth - scaledWidth) / 2
    offsetY = (windowHeight - scaledHeight) / 2
end

--- Recalculate scaling based on current window size
function scaling.update()
    local windowW, windowH = love.graphics.getDimensions()
    scaling.resize(windowW, windowH)
end

--- Begin scaled drawing - call at start of love.draw
function scaling.start()
    -- Push transform for drawing at virtual resolution
    love.graphics.push()

    -- Clear letterbox areas with black
    love.graphics.setColor(0, 0, 0, 1)
    love.graphics.rectangle("fill", 0, 0, actualWidth, actualHeight)

    -- Apply transform: translate to center, then scale
    love.graphics.translate(offsetX, offsetY)
    love.graphics.scale(scale, scale)

    -- Set scissor to prevent drawing outside game area
    love.graphics.setScissor(offsetX, offsetY, virtualWidth * scale, virtualHeight * scale)
end

--- End scaled drawing - call at end of love.draw
function scaling.stop()
    love.graphics.setScissor()
    love.graphics.pop()
end

--- Alias for start() - for compatibility with Fishbowl API
function scaling.apply()
    love.graphics.push()
    love.graphics.translate(offsetX, offsetY)
    love.graphics.scale(scale, scale)
end

--- Alias for stop() - for compatibility with Fishbowl API
function scaling.reset()
    love.graphics.pop()
end

--- Transform screen coordinates to virtual coordinates (for mouse input)
-- @param screenX Screen X coordinate
-- @param screenY Screen Y coordinate
-- @return virtualX, virtualY
function scaling.toVirtual(screenX, screenY)
    local virtualX = (screenX - offsetX) / scale
    local virtualY = (screenY - offsetY) / scale
    return virtualX, virtualY
end

--- Alias for toVirtual() - for compatibility with Fishbowl API
-- @param sx Screen X coordinate
-- @param sy Screen Y coordinate
-- @return gx, gy Game/virtual coordinates
function scaling.toGame(sx, sy)
    return scaling.toVirtual(sx, sy)
end

--- Transform virtual coordinates to screen coordinates
-- @param virtualX Virtual X coordinate
-- @param virtualY Virtual Y coordinate
-- @return screenX, screenY
function scaling.toScreen(virtualX, virtualY)
    local screenX = virtualX * scale + offsetX
    local screenY = virtualY * scale + offsetY
    return screenX, screenY
end

--- Check if screen coordinates are within the game area
-- @param screenX Screen X coordinate
-- @param screenY Screen Y coordinate
-- @return boolean
function scaling.isInBounds(screenX, screenY)
    local vx, vy = scaling.toVirtual(screenX, screenY)
    return vx >= 0 and vx <= virtualWidth and vy >= 0 and vy <= virtualHeight
end

--- Get current scale factor
-- @return scale
function scaling.getScale()
    return scale
end

--- Get letterbox offset
-- @return offsetX, offsetY
function scaling.getOffset()
    return offsetX, offsetY
end

--- Get all transform values at once
-- @return scale, offsetX, offsetY
function scaling.getTransform()
    return scale, offsetX, offsetY
end

--- Get virtual resolution
-- @return width, height
function scaling.getVirtualSize()
    return virtualWidth, virtualHeight
end

--- Alias for getVirtualSize() - for compatibility with Fishbowl API
-- @return width, height
function scaling.getNative()
    return virtualWidth, virtualHeight
end

--- Get actual window size
-- @return width, height
function scaling.getActualSize()
    return actualWidth, actualHeight
end

--- Get the letterbox/pillarbox dimensions
-- @return table {left, right, top, bottom} sizes of black bars
function scaling.getLetterbox()
    return {
        left = offsetX,
        right = actualWidth - (offsetX + virtualWidth * scale),
        top = offsetY,
        bottom = actualHeight - (offsetY + virtualHeight * scale)
    }
end

--- Draw letterbox/pillarbox bars with custom color
-- @param r number Red (0-1)
-- @param g number Green (0-1)
-- @param b number Blue (0-1)
function scaling.drawBars(r, g, b)
    local bars = scaling.getLetterbox()

    love.graphics.setColor(r or 0, g or 0, b or 0, 1)

    -- Left bar
    if bars.left > 0 then
        love.graphics.rectangle("fill", 0, 0, bars.left, actualHeight)
    end
    -- Right bar
    if bars.right > 0 then
        love.graphics.rectangle("fill", actualWidth - bars.right, 0, bars.right, actualHeight)
    end
    -- Top bar
    if bars.top > 0 then
        love.graphics.rectangle("fill", 0, 0, actualWidth, bars.top)
    end
    -- Bottom bar
    if bars.bottom > 0 then
        love.graphics.rectangle("fill", 0, actualHeight - bars.bottom, actualWidth, bars.bottom)
    end

    love.graphics.setColor(1, 1, 1, 1)
end

--- Set up scissor to clip to game area
function scaling.setScissor()
    love.graphics.setScissor(offsetX, offsetY, virtualWidth * scale, virtualHeight * scale)
end

--- Clear scissor
function scaling.clearScissor()
    love.graphics.setScissor()
end

return scaling

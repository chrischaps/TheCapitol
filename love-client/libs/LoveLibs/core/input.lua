--[[
    love2d-input
    Unified input system supporting keyboard and gamepad for LOVE 2D

    Provides a single API for handling input from both keyboard and gamepad,
    with automatic analog stick handling, deadzone support, and action mapping.

    USAGE:
        local input = require("libs.input")

        function love.update(dt)
            local mx, my, isAnalog = input.getMovement()

            if mx ~= 0 or my ~= 0 then
                player.x = player.x + mx * speed * dt
                player.y = player.y + my * speed * dt
            end
        end

        function love.keypressed(key)
            -- Or use the unified action system:
            if input.isActionJustPressed("confirm") then
                -- Handle confirm action
            end
        end

    API:
        input.getMovement() -> moveX, moveY, isAnalog
            Get normalized movement input (-1 to 1)
            Returns analog values from gamepad, digital from keyboard
            isAnalog indicates whether input came from analog stick

        input.isPressed(action) -> boolean
            Check if an action is currently pressed
            Actions: "confirm", "cancel", "pause"

        input.isMoving() -> boolean
            Check if any movement input is active

        input.getMovementMagnitude() -> number (0-1)
            Get movement magnitude for speed scaling

        input.setDeadzone(value)
            Set analog stick deadzone (default 0.2)

        input.getDeadzone() -> number
            Get current deadzone value

        input.hasGamepad() -> boolean
            Check if a gamepad is connected

        input.getGamepad() -> joystick or nil
            Get the first connected gamepad

        input.addAction(name, config)
            Add a custom action mapping
            config = { keys = {"space", "return"}, buttons = {"a"} }

        input.getDirection() -> string or nil
            Get cardinal direction: "up", "down", "left", "right", or nil

    LICENSE: MIT
]]

local input = {}

-- Configuration
local deadzone = 0.2

-- Action mappings
local actions = {
    confirm = { keys = {"return", "space"}, buttons = {"a"} },
    cancel = { keys = {"escape"}, buttons = {"b"} },
    pause = { keys = {"escape", "p"}, buttons = {"start"} }
}

-- Get the first connected gamepad, if any
local function getFirstGamepad()
    local joysticks = love.joystick.getJoysticks()
    for _, joystick in ipairs(joysticks) do
        if joystick:isGamepad() then
            return joystick
        end
    end
    return nil
end

--- Get the first connected gamepad
-- @return joystick or nil
function input.getGamepad()
    return getFirstGamepad()
end

--- Get movement as normalized x, y values (-1 to 1)
-- Returns analog values from gamepad, or digital (-1, 0, 1) from keyboard
-- @return moveX, moveY, isAnalog
function input.getMovement()
    local moveX, moveY = 0, 0

    -- Check gamepad first (analog)
    local gamepad = getFirstGamepad()
    if gamepad then
        local lx = gamepad:getGamepadAxis("leftx") or 0
        local ly = gamepad:getGamepadAxis("lefty") or 0

        -- Apply deadzone
        if math.abs(lx) > deadzone then
            moveX = lx
        end
        if math.abs(ly) > deadzone then
            moveY = ly
        end

        -- If we got gamepad input, return it
        if moveX ~= 0 or moveY ~= 0 then
            return moveX, moveY, true  -- true = analog input
        end
    end

    -- Fall back to keyboard (digital)
    if love.keyboard.isDown("up", "w") then moveY = moveY - 1 end
    if love.keyboard.isDown("down", "s") then moveY = moveY + 1 end
    if love.keyboard.isDown("left", "a") then moveX = moveX - 1 end
    if love.keyboard.isDown("right", "d") then moveX = moveX + 1 end

    -- Normalize diagonal movement for keyboard
    if moveX ~= 0 and moveY ~= 0 then
        local len = math.sqrt(moveX * moveX + moveY * moveY)
        moveX, moveY = moveX / len, moveY / len
    end

    return moveX, moveY, false  -- false = digital input
end

--- Check if an action is currently pressed
-- @param action Action name ("confirm", "cancel", "pause", or custom)
-- @return boolean
function input.isPressed(action)
    local mapping = actions[action]
    if not mapping then return false end

    -- Check keyboard
    if mapping.keys then
        for _, key in ipairs(mapping.keys) do
            if love.keyboard.isDown(key) then
                return true
            end
        end
    end

    -- Check gamepad
    local gamepad = getFirstGamepad()
    if gamepad and mapping.buttons then
        for _, button in ipairs(mapping.buttons) do
            if gamepad:isGamepadDown(button) then
                return true
            end
        end
    end

    return false
end

--- Check if any movement input is active
-- @return boolean
function input.isMoving()
    local mx, my = input.getMovement()
    return mx ~= 0 or my ~= 0
end

--- Get movement magnitude (0 to 1)
-- @return number
function input.getMovementMagnitude()
    local mx, my = input.getMovement()
    return math.min(1, math.sqrt(mx * mx + my * my))
end

--- Set deadzone for analog sticks
-- @param value Deadzone value (0-1, default 0.2)
function input.setDeadzone(value)
    deadzone = math.max(0, math.min(1, value))
end

--- Get current deadzone value
-- @return number
function input.getDeadzone()
    return deadzone
end

--- Check if a gamepad is connected
-- @return boolean
function input.hasGamepad()
    return getFirstGamepad() ~= nil
end

--- Add a custom action mapping
-- @param name Action name
-- @param config Table with keys and/or buttons arrays
function input.addAction(name, config)
    actions[name] = config
end

--- Remove an action mapping
-- @param name Action name
function input.removeAction(name)
    actions[name] = nil
end

--- Get all action mappings
-- @return table
function input.getActions()
    local copy = {}
    for k, v in pairs(actions) do
        copy[k] = v
    end
    return copy
end

--- Get cardinal direction from current movement
-- @return string or nil ("up", "down", "left", "right")
function input.getDirection()
    local mx, my = input.getMovement()

    if mx == 0 and my == 0 then
        return nil
    end

    -- Determine dominant axis
    if math.abs(mx) > math.abs(my) then
        return mx > 0 and "right" or "left"
    else
        return my > 0 and "down" or "up"
    end
end

--- Get 8-directional movement string
-- @return string or nil ("up", "down", "left", "right", "up-left", etc.)
function input.getDirection8()
    local mx, my = input.getMovement()

    if mx == 0 and my == 0 then
        return nil
    end

    local threshold = 0.3
    local horizontal = mx > threshold and "right" or (mx < -threshold and "left" or nil)
    local vertical = my > threshold and "down" or (my < -threshold and "up" or nil)

    if horizontal and vertical then
        return vertical .. "-" .. horizontal
    end

    return horizontal or vertical
end

return input

--[[
    love2d-scenes
    Simple scene management system for LOVE 2D

    Manages scene lifecycle, transitions between scenes, and automatic
    delegation of LOVE callbacks to the current scene.

    USAGE:
        local scenes = require("libs.scenes")
        local MenuScene = require("scenes.menu")
        local GameScene = require("scenes.game")

        function love.load()
            -- Register scenes
            scenes.register("menu", MenuScene)
            scenes.register("game", GameScene)

            -- Start with menu
            scenes.switchTo("menu")
        end

        -- Delegate all LOVE callbacks
        function love.update(dt) scenes.update(dt) end
        function love.draw() scenes.draw() end
        function love.keypressed(...) scenes.keypressed(...) end
        function love.mousepressed(...) scenes.mousepressed(...) end
        function love.gamepadpressed(...) scenes.gamepadpressed(...) end

    SCENE FORMAT:
        A scene is a table with optional lifecycle methods:

        local MyScene = {}

        function MyScene:load(...)           -- Called when scene becomes active
        function MyScene:unload()            -- Called when scene is switched away
        function MyScene:update(dt)          -- Called every frame
        function MyScene:draw()              -- Called every frame after update
        function MyScene:keypressed(key, scancode, isrepeat)
        function MyScene:keyreleased(key, scancode)
        function MyScene:mousepressed(x, y, button, istouch, presses)
        function MyScene:mousereleased(x, y, button, istouch, presses)
        function MyScene:mousemoved(x, y, dx, dy, istouch)
        function MyScene:wheelmoved(x, y)
        function MyScene:gamepadpressed(joystick, button)
        function MyScene:gamepadreleased(joystick, button)
        function MyScene:resize(w, h)
        function MyScene:focus(focused)
        function MyScene:visible(visible)

        return MyScene

    API:
        scenes.register(name, scene)
            Register a scene module with a name

        scenes.switchTo(name, ...)
            Switch to a registered scene, passing args to load()

        scenes.setScene(sceneModule, ...)
            Set scene directly from a module (without registration)

        scenes.getCurrent() -> scene or nil
            Get the current scene module

        scenes.getCurrentName() -> string or nil
            Get the current scene name (if registered)

        scenes.push(name, ...)
            Push current scene to stack and switch to new scene

        scenes.pop()
            Pop scene stack and return to previous scene

        scenes.getStackDepth() -> number
            Get number of scenes on the stack

        -- LOVE callback delegates:
        scenes.update(dt)
        scenes.draw()
        scenes.keypressed(key, scancode, isrepeat)
        scenes.keyreleased(key, scancode)
        scenes.mousepressed(x, y, button, istouch, presses)
        scenes.mousereleased(x, y, button, istouch, presses)
        scenes.mousemoved(x, y, dx, dy, istouch)
        scenes.wheelmoved(x, y)
        scenes.gamepadpressed(joystick, button)
        scenes.gamepadreleased(joystick, button)
        scenes.resize(w, h)
        scenes.focus(focused)
        scenes.visible(visible)

    LICENSE: MIT
]]

local scenes = {}

-- Internal state
local registry = {}
local current = nil
local currentName = nil
local sceneStack = {}

--------------------------------------------------
-- UTILITIES
--------------------------------------------------

-- Safely call a method on a scene if it exists
local function safeCall(target, methodName, ...)
    if target and target[methodName] then
        return target[methodName](target, ...)
    end
end

--------------------------------------------------
-- PUBLIC API
--------------------------------------------------

--- Register a scene module with a name
-- @param name Scene name
-- @param scene Scene module/table
function scenes.register(name, scene)
    registry[name] = scene
end

--- Unregister a scene
-- @param name Scene name
function scenes.unregister(name)
    registry[name] = nil
end

--- Check if a scene is registered
-- @param name Scene name
-- @return boolean
function scenes.isRegistered(name)
    return registry[name] ~= nil
end

--- Get list of registered scene names
-- @return table
function scenes.getRegistered()
    local list = {}
    for name, _ in pairs(registry) do
        table.insert(list, name)
    end
    return list
end

--- Switch to a registered scene by name
-- @param name Scene name
-- @param ... Arguments to pass to scene:load()
function scenes.switchTo(name, ...)
    local scene = registry[name]
    if not scene then
        error("Scene not registered: " .. tostring(name))
    end

    -- Unload current scene
    safeCall(current, "unload")

    -- Switch and load new scene
    current = scene
    currentName = name
    safeCall(current, "load", ...)
end

--- Set scene directly from a module (without registration)
-- @param sceneModule Scene module/table
-- @param ... Arguments to pass to scene:load()
function scenes.setScene(sceneModule, ...)
    -- Unload current scene
    safeCall(current, "unload")

    -- Switch and load new scene
    current = sceneModule
    currentName = nil  -- Not a registered scene
    safeCall(current, "load", ...)
end

--- Get the current scene module
-- @return scene or nil
function scenes.getCurrent()
    return current
end

--- Get the current scene name (if registered)
-- @return string or nil
function scenes.getCurrentName()
    return currentName
end

--- Push current scene to stack and switch to new scene
-- @param name Scene name
-- @param ... Arguments to pass to scene:load()
function scenes.push(name, ...)
    if current then
        table.insert(sceneStack, { scene = current, name = currentName })
        safeCall(current, "pause")  -- Optional pause callback
    end
    scenes.switchTo(name, ...)
end

--- Pop scene stack and return to previous scene
-- @param ... Arguments to pass to scene:resume()
-- @return boolean success
function scenes.pop(...)
    if #sceneStack == 0 then
        return false
    end

    -- Unload current scene
    safeCall(current, "unload")

    -- Restore previous scene
    local previous = table.remove(sceneStack)
    current = previous.scene
    currentName = previous.name
    safeCall(current, "resume", ...)  -- Optional resume callback

    return true
end

--- Get number of scenes on the stack
-- @return number
function scenes.getStackDepth()
    return #sceneStack
end

--- Clear the scene stack
function scenes.clearStack()
    sceneStack = {}
end

--------------------------------------------------
-- LOVE CALLBACK DELEGATES
--------------------------------------------------

function scenes.update(dt)
    safeCall(current, "update", dt)
end

function scenes.draw()
    safeCall(current, "draw")
end

function scenes.keypressed(key, scancode, isrepeat)
    safeCall(current, "keypressed", key, scancode, isrepeat)
end

function scenes.keyreleased(key, scancode)
    safeCall(current, "keyreleased", key, scancode)
end

function scenes.mousepressed(x, y, button, istouch, presses)
    safeCall(current, "mousepressed", x, y, button, istouch, presses)
end

function scenes.mousereleased(x, y, button, istouch, presses)
    safeCall(current, "mousereleased", x, y, button, istouch, presses)
end

function scenes.mousemoved(x, y, dx, dy, istouch)
    safeCall(current, "mousemoved", x, y, dx, dy, istouch)
end

function scenes.wheelmoved(x, y)
    safeCall(current, "wheelmoved", x, y)
end

function scenes.gamepadpressed(joystick, button)
    safeCall(current, "gamepadpressed", joystick, button)
end

function scenes.gamepadreleased(joystick, button)
    safeCall(current, "gamepadreleased", joystick, button)
end

function scenes.resize(w, h)
    safeCall(current, "resize", w, h)
end

function scenes.focus(focused)
    safeCall(current, "focus", focused)
end

function scenes.visible(visible)
    safeCall(current, "visible", visible)
end

function scenes.textinput(text)
    safeCall(current, "textinput", text)
end

function scenes.touchpressed(id, x, y, dx, dy, pressure)
    safeCall(current, "touchpressed", id, x, y, dx, dy, pressure)
end

function scenes.touchreleased(id, x, y, dx, dy, pressure)
    safeCall(current, "touchreleased", id, x, y, dx, dy, pressure)
end

function scenes.touchmoved(id, x, y, dx, dy, pressure)
    safeCall(current, "touchmoved", id, x, y, dx, dy, pressure)
end

return scenes

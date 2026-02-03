-- The Capitol - Love2D Client
-- Entry point

local scenes = require("libs.LoveLibs.core.scenes")
local scaling = require("libs.LoveLibs.graphics.scaling")
local config = require("src.config")
local auth = require("src.state.auth")

-- Load scenes
local loginScene = require("src.scenes.login")
local loadingScene = require("src.scenes.loading")
local gameScene = require("src.scenes.game")

function love.load()
    -- Set up scaling for virtual resolution
    scaling.init(config.VIRTUAL_WIDTH, config.VIRTUAL_HEIGHT)

    -- Initialize authentication
    auth.init()

    -- Register scenes
    scenes.register("login", loginScene)
    scenes.register("loading", loadingScene)
    scenes.register("game", gameScene)

    -- Start with login or try to reconnect if we have a token
    if auth.isAuthenticated() then
        scenes.switchTo("loading")
    else
        scenes.switchTo("login")
    end

    -- Set default font
    love.graphics.setNewFont(14)
end

function love.update(dt)
    scenes.update(dt)
end

function love.draw()
    scaling.start()
    scenes.draw()
    scaling.stop()
end

function love.resize(w, h)
    scaling.resize(w, h)
    scenes.resize(w, h)
end

function love.keypressed(key, scancode, isrepeat)
    scenes.keypressed(key, scancode, isrepeat)
end

function love.keyreleased(key, scancode)
    scenes.keyreleased(key, scancode)
end

function love.mousepressed(x, y, button, istouch, presses)
    scenes.mousepressed(x, y, button, istouch, presses)
end

function love.mousereleased(x, y, button, istouch, presses)
    scenes.mousereleased(x, y, button, istouch, presses)
end

function love.mousemoved(x, y, dx, dy, istouch)
    scenes.mousemoved(x, y, dx, dy, istouch)
end

function love.wheelmoved(x, y)
    scenes.wheelmoved(x, y)
end

function love.textinput(text)
    scenes.textinput(text)
end

function love.focus(focused)
    scenes.focus(focused)
end

function love.visible(visible)
    scenes.visible(visible)
end

-- Love2D configuration for The Capitol
function love.conf(t)
    t.identity = "thecapitol"
    t.version = "11.4"
    t.console = true -- Enable console for debugging on Windows

    t.window.title = "The Capitol"
    t.window.width = 1280
    t.window.height = 720
    t.window.resizable = true
    t.window.minwidth = 800
    t.window.minheight = 600
    t.window.vsync = 1

    t.modules.joystick = false
    t.modules.physics = false
    t.modules.video = false
end

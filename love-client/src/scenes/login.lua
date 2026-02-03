-- Login/Register scene
local config = require("src.config")
local scenes = require("libs.LoveLibs.core.scenes")
local scaling = require("libs.LoveLibs.graphics.scaling")
local api = require("src.net.api")
local auth = require("src.state.auth")
local Button = require("src.ui.button")
local Input = require("src.ui.input")

local login = {}

-- State
local mode = "login" -- "login" or "register"
local emailInput = nil
local passwordInput = nil
local nameInput = nil
local submitButton = nil
local toggleButton = nil
local errorMessage = nil
local isSubmitting = false

function login:load()
    mode = "login"
    errorMessage = nil
    isSubmitting = false

    local vw, vh = scaling.getVirtualSize()
    local centerX = vw / 2
    local inputWidth = 300
    local inputHeight = 40
    local startY = vh / 2 - 80

    emailInput = Input.new(centerX - inputWidth / 2, startY, inputWidth, inputHeight, "Email")
    passwordInput = Input.new(centerX - inputWidth / 2, startY + 60, inputWidth, inputHeight, "Password")
    passwordInput.isPassword = true
    nameInput = Input.new(centerX - inputWidth / 2, startY + 120, inputWidth, inputHeight, "Display Name")

    submitButton = Button.new(centerX - inputWidth / 2, startY + 180, inputWidth, inputHeight, "Login", function()
        self:submit()
    end)

    toggleButton = Button.new(centerX - inputWidth / 2, startY + 240, inputWidth, 30, "Need an account? Register", function()
        self:toggleMode()
    end)

    emailInput:focus()
end

function login:toggleMode()
    if mode == "login" then
        mode = "register"
        submitButton.text = "Register"
        toggleButton.text = "Have an account? Login"
    else
        mode = "login"
        submitButton.text = "Login"
        toggleButton.text = "Need an account? Register"
    end
    errorMessage = nil
end

function login:submit()
    if isSubmitting then return end

    local email = emailInput:getValue()
    local password = passwordInput:getValue()

    if email == "" or password == "" then
        errorMessage = "Please fill in all fields"
        return
    end

    if mode == "register" then
        local name = nameInput:getValue()
        if name == "" then
            errorMessage = "Please enter a display name"
            return
        end
    end

    isSubmitting = true
    errorMessage = nil
    submitButton.enabled = false

    -- Run in coroutine to avoid blocking
    local co = coroutine.create(function()
        local response
        if mode == "login" then
            response = api.login(email, password)
        else
            local name = nameInput:getValue()
            response = api.register(email, password, name)
        end

        isSubmitting = false
        submitButton.enabled = true

        if response.ok and response.data then
            if response.data.token then
                auth.setAuth(response.data.token, response.data.player_id or "")
                scenes.switchTo("loading")
            else
                errorMessage = "Invalid response from server"
            end
        else
            if response.data and response.data.error then
                errorMessage = response.data.error
            else
                errorMessage = "Connection failed"
            end
        end
    end)

    coroutine.resume(co)
end

function login:update(dt)
    local mx, my = scaling.toVirtual(love.mouse.getPosition())

    emailInput:update(dt)
    passwordInput:update(dt)
    if mode == "register" then
        nameInput:update(dt)
    end
    submitButton:update(mx, my)
    toggleButton:update(mx, my)
end

function login:keypressed(key)
    if key == "tab" then
        -- Cycle focus
        if emailInput.isFocused then
            emailInput:unfocus()
            passwordInput:focus()
        elseif passwordInput.isFocused then
            passwordInput:unfocus()
            if mode == "register" then
                nameInput:focus()
            else
                emailInput:focus()
            end
        elseif nameInput.isFocused then
            nameInput:unfocus()
            emailInput:focus()
        end
        return
    end

    if key == "return" or key == "kpenter" then
        self:submit()
        return
    end

    emailInput:keypressed(key)
    passwordInput:keypressed(key)
    if mode == "register" then
        nameInput:keypressed(key)
    end
end

function login:textinput(text)
    emailInput:textinput(text)
    passwordInput:textinput(text)
    if mode == "register" then
        nameInput:textinput(text)
    end
end

function login:mousepressed(x, y, button)
    local vx, vy = scaling.toVirtual(x, y)

    emailInput:mousepressed(vx, vy, button)
    passwordInput:mousepressed(vx, vy, button)
    if mode == "register" then
        nameInput:mousepressed(vx, vy, button)
    end
    submitButton:mousepressed(vx, vy, button)
    toggleButton:mousepressed(vx, vy, button)
end

function login:mousereleased(x, y, button)
    local vx, vy = scaling.toVirtual(x, y)

    submitButton:mousereleased(vx, vy, button)
    toggleButton:mousereleased(vx, vy, button)
end

function login:draw()
    local colors = config.COLORS
    local vw, vh = scaling.getVirtualSize()

    -- Background
    love.graphics.setColor(colors.BACKGROUND)
    love.graphics.rectangle("fill", 0, 0, vw, vh)

    -- Title
    love.graphics.setColor(colors.GOLDEN)
    local title = "THE CAPITOL"
    local font = love.graphics.getFont()
    local titleWidth = font:getWidth(title)
    love.graphics.print(title, (vw - titleWidth) / 2, 100)

    -- Subtitle
    love.graphics.setColor(colors.TEXT_DIM)
    local subtitle = mode == "login" and "Welcome back" or "Create your account"
    local subtitleWidth = font:getWidth(subtitle)
    love.graphics.print(subtitle, (vw - subtitleWidth) / 2, 130)

    -- Input fields
    emailInput:draw()
    passwordInput:draw()
    if mode == "register" then
        nameInput:draw()
        -- Adjust button positions
        submitButton:setPosition(submitButton.x, nameInput.y + 60)
        toggleButton:setPosition(toggleButton.x, nameInput.y + 120)
    else
        submitButton:setPosition(submitButton.x, passwordInput.y + 60)
        toggleButton:setPosition(toggleButton.x, passwordInput.y + 120)
    end

    -- Buttons
    submitButton:draw()
    toggleButton:draw()

    -- Error message
    if errorMessage then
        love.graphics.setColor(colors.QUALITY_F)
        local errorWidth = font:getWidth(errorMessage)
        love.graphics.print(errorMessage, (vw - errorWidth) / 2, toggleButton.y + 50)
    end

    -- Loading indicator
    if isSubmitting then
        love.graphics.setColor(colors.GOLDEN)
        love.graphics.print("Connecting...", (vw - font:getWidth("Connecting...")) / 2, toggleButton.y + 50)
    end
end

return login

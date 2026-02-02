--[[
    Particles - Ambient Floating Particle System for LÖVE2D

    A configurable particle system for atmospheric effects like dust motes,
    debris, snow, embers, etc. Supports multiple depth layers and optional
    light ray interaction.

    Usage:
        local Particles = require("LoveLibs.graphics.particles")

        function love.load()
            Particles.init(1280, 720, {
                backCount = 30,
                frontCount = 20,
                colors = {
                    normal = {1, 0.9, 0.8},
                    lit = {1, 0.95, 0.85}
                }
            })
        end

        function love.update(dt)
            Particles.update(dt, screenW, screenH)
        end

        function love.draw()
            Particles.drawBack()   -- Behind game objects
            -- Draw game objects
            Particles.drawFront()  -- In front of game objects
        end
]]

local Particles = {}

-- Particle storage
local backParticles = {}
local frontParticles = {}

-- Light ray data (optional, for particles that glow in light)
local lightRays = {}

-- Time tracking
local time = 0

-- Default configuration
local config = {
    -- Particle counts
    backCount = 35,
    frontCount = 20,

    -- Size range
    minSize = 0.5,
    maxSize = 2.5,

    -- Movement
    minSpeed = 2,
    maxSpeed = 15,
    driftStrength = 8,

    -- Visual
    sparkleChance = 0.02,

    -- Colors (can be overridden)
    colors = {
        normal = {0.9, 0.9, 0.95},  -- Default particle color
        lit = {1, 0.98, 0.9},       -- Color when in light ray
        glow = {1, 1, 1}            -- Glow color for larger particles
    },

    -- Bounds
    bottomMargin = 80,  -- Keep particles above this from bottom
}

--============================================================================
-- INTERNAL FUNCTIONS
--============================================================================

local function createParticle(screenW, screenH, forFront)
    local size = config.minSize + math.random() * (config.maxSize - config.minSize)
    local baseAlpha = forFront and (0.15 + math.random() * 0.15) or (0.08 + math.random() * 0.12)

    return {
        x = math.random() * screenW,
        y = math.random() * (screenH - config.bottomMargin),
        size = size,
        baseAlpha = baseAlpha,
        alpha = baseAlpha,

        -- Drift velocity
        vx = (math.random() - 0.5) * config.driftStrength,
        vy = config.minSpeed + math.random() * (config.maxSpeed - config.minSpeed),

        -- Animation
        phase = math.random() * math.pi * 2,
        pulseSpeed = 0.5 + math.random() * 1.5,
        wobbleSpeed = 0.3 + math.random() * 0.7,
        wobbleAmount = 5 + math.random() * 15,

        -- Visual
        brightness = 0.7 + math.random() * 0.3,
        inLightRay = false,

        -- Depth (affects parallax movement)
        depth = forFront and (0.8 + math.random() * 0.2) or (0.3 + math.random() * 0.4),
    }
end

local function isInLightRay(x, y, screenH)
    for _, ray in ipairs(lightRays) do
        local rayProgress = y / screenH
        local rayX = ray.x + math.sin(time * (ray.speed or 0.1) + (ray.offset or 0)) * 50
        local rayWidth = ray.width or 80
        local rayWidthAtY = rayWidth * (1 + rayProgress * 0.5)
        local rayLeftAtY = rayX - rayWidthAtY * 0.3 * rayProgress
        local rayRightAtY = rayX + rayWidthAtY * (1 + 0.5 * rayProgress)

        if x >= rayLeftAtY and x <= rayRightAtY then
            return true
        end
    end
    return false
end

local function updateParticle(p, dt, screenW, screenH)
    -- Gentle wobble motion
    local wobble = math.sin(time * p.wobbleSpeed + p.phase) * p.wobbleAmount * dt

    -- Update position with drift
    p.x = p.x + p.vx * dt + wobble
    p.y = p.y + p.vy * dt * p.depth

    -- Alpha pulse
    local pulse = math.sin(time * p.pulseSpeed + p.phase)
    p.alpha = p.baseAlpha * (0.7 + 0.3 * pulse)

    -- Light ray interaction
    if #lightRays > 0 then
        p.inLightRay = isInLightRay(p.x, p.y, screenH)
        if p.inLightRay then
            p.alpha = p.alpha * 2.5
            p.brightness = math.min(1, p.brightness + dt * 2)
        else
            p.brightness = math.max(0.7, p.brightness - dt * 0.5)
        end
    end

    -- Occasional sparkle
    if math.random() < config.sparkleChance * dt * 60 then
        p.alpha = math.min(0.8, p.alpha * 3)
    end

    -- Wrap around screen edges
    if p.x < -10 then p.x = screenW + 10 end
    if p.x > screenW + 10 then p.x = -10 end
    if p.y > screenH - config.bottomMargin + 10 then
        p.y = -10
        p.x = math.random() * screenW
    end
    if p.y < -10 then
        p.y = screenH - config.bottomMargin
        p.x = math.random() * screenW
    end
end

local function drawParticle(p)
    local brightness = p.brightness
    local colors = config.colors

    if p.inLightRay then
        love.graphics.setColor(colors.lit[1], colors.lit[2], colors.lit[3], p.alpha)
    else
        love.graphics.setColor(
            colors.normal[1] * brightness,
            colors.normal[2] * brightness,
            colors.normal[3] * brightness,
            p.alpha
        )
    end

    love.graphics.circle("fill", p.x, p.y, p.size)

    -- Glow for larger particles
    if p.size > 1.5 then
        love.graphics.setColor(colors.glow[1], colors.glow[2], colors.glow[3], p.alpha * 0.3)
        love.graphics.circle("fill", p.x, p.y, p.size * 1.5)
    end
end

--============================================================================
-- PUBLIC API
--============================================================================

--- Initialize particle system
--- @param screenW number Screen width
--- @param screenH number Screen height
--- @param userConfig table|nil Optional configuration overrides
function Particles.init(screenW, screenH, userConfig)
    -- Merge user config
    if userConfig then
        for key, value in pairs(userConfig) do
            if type(value) == "table" and type(config[key]) == "table" then
                for k, v in pairs(value) do
                    config[key][k] = v
                end
            else
                config[key] = value
            end
        end
    end

    -- Create particles
    backParticles = {}
    frontParticles = {}

    for i = 1, config.backCount do
        table.insert(backParticles, createParticle(screenW, screenH, false))
    end

    for i = 1, config.frontCount do
        table.insert(frontParticles, createParticle(screenW, screenH, true))
    end
end

--- Set light rays for particle interaction
--- @param rays table Array of ray objects with {x, width, speed, offset}
function Particles.setLightRays(rays)
    lightRays = rays or {}
end

--- Clear light rays
function Particles.clearLightRays()
    lightRays = {}
end

--- Update all particles
--- @param dt number Delta time
--- @param screenW number Screen width
--- @param screenH number Screen height
function Particles.update(dt, screenW, screenH)
    time = time + dt

    for _, p in ipairs(backParticles) do
        updateParticle(p, dt, screenW, screenH)
    end

    for _, p in ipairs(frontParticles) do
        updateParticle(p, dt, screenW, screenH)
    end
end

--- Draw back layer particles (behind game objects)
function Particles.drawBack()
    for _, p in ipairs(backParticles) do
        drawParticle(p)
    end
end

--- Draw front layer particles (in front of game objects)
function Particles.drawFront()
    for _, p in ipairs(frontParticles) do
        drawParticle(p)
    end
end

--- Draw all particles (single layer mode)
function Particles.drawAll()
    Particles.drawBack()
    Particles.drawFront()
end

--- Get current configuration
--- @return table Current configuration
function Particles.getConfig()
    return config
end

--- Update configuration at runtime
--- @param newConfig table Configuration values to update
function Particles.setConfig(newConfig)
    for key, value in pairs(newConfig) do
        if type(value) == "table" and type(config[key]) == "table" then
            for k, v in pairs(value) do
                config[key][k] = v
            end
        else
            config[key] = value
        end
    end
end

--- Set particle colors
--- @param normal table|nil {r, g, b} Normal color
--- @param lit table|nil {r, g, b} Lit color (in light rays)
--- @param glow table|nil {r, g, b} Glow color
function Particles.setColors(normal, lit, glow)
    if normal then config.colors.normal = normal end
    if lit then config.colors.lit = lit end
    if glow then config.colors.glow = glow end
end

--- Get particle counts
--- @return number, number Back count, front count
function Particles.getCounts()
    return #backParticles, #frontParticles
end

--- Spawn additional particles (burst effect)
--- @param count number Number of particles to spawn
--- @param x number X position
--- @param y number Y position
--- @param screenW number Screen width
--- @param screenH number Screen height
--- @param front boolean|nil Spawn in front layer (default false)
function Particles.burst(count, x, y, screenW, screenH, front)
    local target = front and frontParticles or backParticles

    for i = 1, count do
        local p = createParticle(screenW, screenH, front or false)
        p.x = x + (math.random() - 0.5) * 50
        p.y = y + (math.random() - 0.5) * 50
        p.alpha = p.baseAlpha * 2
        table.insert(target, p)
    end
end

return Particles

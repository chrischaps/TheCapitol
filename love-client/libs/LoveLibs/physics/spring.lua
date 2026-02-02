--[[
    Spring Physics Library
    ======================
    Spring-damper connections using Hooke's law.
    Ideal for soft bodies, bouncy physics, and elastic connections.

    Usage:
        local Spring = require('LoveLibs.physics.spring')
        local Verlet = require('LoveLibs.physics.verlet')

        local point_a = Verlet.Point(100, 100)
        local point_b = Verlet.Point(200, 100)

        local spring = Spring.new(point_a, point_b, {
            stiffness = 200,   -- Higher = stiffer
            damping = 10,      -- Higher = more damping
            rest_length = 100  -- Optional, defaults to current distance
        })

        -- In update loop:
        spring:apply_forces()
        point_a:update(dt, gravity)
        point_b:update(dt, gravity)

    Requires: LoveLibs/physics/vec2.lua
]]

-- Try to find vec2 in the same directory or parent paths
local vec2
local success
success, vec2 = pcall(require, 'LoveLibs.physics.vec2')
if not success then
    success, vec2 = pcall(require, 'physics.vec2')
end
if not success then
    success, vec2 = pcall(require, 'lib.vec2')
end
if not success then
    error("Could not find vec2 library. Tried: LoveLibs.physics.vec2, physics.vec2, lib.vec2")
end

local Spring = {}
Spring.__index = Spring

function Spring.new(point_a, point_b, options)
    options = options or {}
    local self = setmetatable({}, Spring)

    self.point_a = point_a
    self.point_b = point_b

    -- Rest length defaults to current distance between points
    self.rest_length = options.rest_length or vec2.distance(point_a.pos, point_b.pos)

    -- Spring stiffness (Hooke's constant k) - higher = stiffer
    self.stiffness = options.stiffness or 200

    -- Damping coefficient (c) - higher = more damping
    self.damping = options.damping or 10

    return self
end

function Spring:apply_forces()
    local delta = vec2.sub(self.point_b.pos, self.point_a.pos)
    local current_length = vec2.length(delta)

    if current_length == 0 then return end

    local direction = vec2.normalize(delta)

    -- Spring force (Hooke's law): F = -k * (length - rest_length)
    local displacement = current_length - self.rest_length
    local spring_force = self.stiffness * displacement

    -- Damping force: F = -c * relative_velocity_along_spring
    local vel_a = self.point_a:get_velocity()
    local vel_b = self.point_b:get_velocity()
    local relative_vel = vec2.sub(vel_b, vel_a)
    local damping_force = self.damping * vec2.dot(relative_vel, direction)

    -- Combined force along spring direction
    local total_force = spring_force + damping_force
    local force_vec = vec2.mul(direction, total_force)

    -- Apply equal and opposite forces (Newton's third law)
    if not self.point_a.pinned then
        self.point_a:apply_force(force_vec)
    end
    if not self.point_b.pinned then
        self.point_b:apply_force(vec2.mul(force_vec, -1))
    end
end

-- Get current stretch ratio: 0 = at rest, positive = stretched, negative = compressed
function Spring:get_stretch()
    local current_length = vec2.distance(self.point_a.pos, self.point_b.pos)
    return (current_length - self.rest_length) / self.rest_length
end

-- Get current length
function Spring:get_length()
    return vec2.distance(self.point_a.pos, self.point_b.pos)
end

-- Set new rest length
function Spring:set_rest_length(length)
    self.rest_length = length
end

-- Set rest length to current length
function Spring:relax()
    self.rest_length = self:get_length()
end

-- Drawing functions (require LÖVE)
function Spring:draw(color)
    if not love or not love.graphics then return end

    local stretch = self:get_stretch()

    if color then
        love.graphics.setColor(color)
    else
        -- Color based on stretch: green = rest, red = stretched, blue = compressed
        if stretch > 0 then
            love.graphics.setColor(1, 1 - math.min(stretch, 1), 0)
        else
            love.graphics.setColor(0, 1 - math.min(-stretch, 1), 1)
        end
    end

    love.graphics.line(
        self.point_a.pos.x, self.point_a.pos.y,
        self.point_b.pos.x, self.point_b.pos.y
    )
end

-- Draw as a coiled spring visualization
function Spring:draw_coil(segments, amplitude, color)
    if not love or not love.graphics then return end

    segments = segments or 10
    amplitude = amplitude or 5

    local start = self.point_a.pos
    local finish = self.point_b.pos
    local delta = vec2.sub(finish, start)
    local length = vec2.length(delta)

    if length == 0 then return end

    if color then
        love.graphics.setColor(color)
    else
        local stretch = self:get_stretch()
        if stretch > 0 then
            love.graphics.setColor(1, 1 - math.min(stretch, 1), 0)
        else
            love.graphics.setColor(0, 1 - math.min(-stretch, 1), 1)
        end
    end

    local dir = vec2.normalize(delta)
    local perp = vec2.perpendicular(dir)

    local points = {}
    for i = 0, segments do
        local t = i / segments
        local base = vec2.add(start, vec2.mul(delta, t))

        -- Zigzag pattern (skip first and last)
        local offset = 0
        if i > 0 and i < segments then
            offset = (i % 2 == 0) and amplitude or -amplitude
        end

        local pos = vec2.add(base, vec2.mul(perp, offset))
        table.insert(points, pos.x)
        table.insert(points, pos.y)
    end

    if #points >= 4 then
        love.graphics.line(points)
    end
end

--------------------------------------------------------------------------------
-- SpringSystem: Manage multiple springs efficiently
--------------------------------------------------------------------------------

local SpringSystem = {}
SpringSystem.__index = SpringSystem

function SpringSystem.new()
    local self = setmetatable({}, SpringSystem)
    self.springs = {}
    return self
end

function SpringSystem:add(spring)
    table.insert(self.springs, spring)
    return spring
end

function SpringSystem:create(point_a, point_b, options)
    local spring = Spring.new(point_a, point_b, options)
    return self:add(spring)
end

function SpringSystem:remove(spring)
    for i, s in ipairs(self.springs) do
        if s == spring then
            table.remove(self.springs, i)
            return true
        end
    end
    return false
end

function SpringSystem:apply_forces()
    for _, spring in ipairs(self.springs) do
        spring:apply_forces()
    end
end

function SpringSystem:draw(color)
    for _, spring in ipairs(self.springs) do
        spring:draw(color)
    end
end

function SpringSystem:draw_coils(segments, amplitude, color)
    for _, spring in ipairs(self.springs) do
        spring:draw_coil(segments, amplitude, color)
    end
end

--------------------------------------------------------------------------------
-- Module exports
--------------------------------------------------------------------------------

return {
    new = Spring.new,
    Spring = Spring,
    System = SpringSystem.new,
    SpringSystem = SpringSystem
}

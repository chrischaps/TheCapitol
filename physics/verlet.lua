--[[
    Verlet Physics Library
    ======================
    Position-based physics simulation using Verlet integration.
    Stable, efficient, and ideal for ropes, chains, cloth, and soft bodies.

    Usage:
        local Verlet = require('LoveLibs.physics.verlet')

        -- Create a point
        local point = Verlet.Point(100, 200, { mass = 1, friction = 0.99 })

        -- Create a chain (rope/tentacle)
        local chain = Verlet.Chain(100, 50, 10, 15)  -- x, y, segments, segment_length

        -- Update physics
        local gravity = {x = 0, y = 980}
        point:update(dt, gravity)
        chain:update(dt, gravity)

    Requires: LoveLibs/physics/vec2.lua (2D vector math)
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

local Verlet = {}

--------------------------------------------------------------------------------
-- VerletPoint: A single physics point with position-based dynamics
--------------------------------------------------------------------------------

local VerletPoint = {}
VerletPoint.__index = VerletPoint

function VerletPoint.new(x, y, options)
    options = options or {}
    local self = setmetatable({}, VerletPoint)

    self.pos = vec2.new(x, y)
    self.old_pos = vec2.new(x, y)
    self.acceleration = vec2.new(0, 0)

    self.pinned = options.pinned or false
    self.friction = options.friction or 0.99
    self.mass = options.mass or 1
    self.radius = options.radius or 5

    return self
end

function VerletPoint:apply_force(force)
    -- F = ma, so a = F/m
    self.acceleration = vec2.add(self.acceleration, vec2.div(force, self.mass))
end

function VerletPoint:update(dt, gravity)
    if self.pinned then
        self.acceleration = vec2.new(0, 0)
        return
    end

    -- Apply gravity
    if gravity then
        local g = type(gravity) == "table" and gravity.x and gravity or vec2.new(gravity.x or 0, gravity.y or 0)
        self:apply_force(vec2.mul(g, self.mass))
    end

    -- Verlet integration: new_pos = pos + (pos - old_pos) * friction + acceleration * dt²
    local velocity = vec2.sub(self.pos, self.old_pos)
    velocity = vec2.mul(velocity, self.friction)

    self.old_pos = vec2.clone(self.pos)
    self.pos = vec2.add(self.pos, velocity)
    self.pos = vec2.add(self.pos, vec2.mul(self.acceleration, dt * dt))

    self.acceleration = vec2.new(0, 0)
end

function VerletPoint:set_position(x, y)
    self.pos.x, self.pos.y = x, y
    self.old_pos.x, self.old_pos.y = x, y
end

function VerletPoint:move_to(x, y)
    -- Preserves velocity
    local dx, dy = x - self.pos.x, y - self.pos.y
    self.pos.x, self.pos.y = x, y
    self.old_pos.x = self.old_pos.x + dx
    self.old_pos.y = self.old_pos.y + dy
end

function VerletPoint:get_velocity()
    return vec2.sub(self.pos, self.old_pos)
end

function VerletPoint:set_velocity(vel)
    self.old_pos = vec2.sub(self.pos, vel)
end

function VerletPoint:constrain_to_bounds(min_x, min_y, max_x, max_y, bounce)
    bounce = bounce or 0.5
    local vel = self:get_velocity()

    if self.pos.x < min_x then
        self.pos.x = min_x
        self.old_pos.x = self.pos.x + vel.x * bounce
    elseif self.pos.x > max_x then
        self.pos.x = max_x
        self.old_pos.x = self.pos.x + vel.x * bounce
    end

    if self.pos.y < min_y then
        self.pos.y = min_y
        self.old_pos.y = self.pos.y + vel.y * bounce
    elseif self.pos.y > max_y then
        self.pos.y = max_y
        self.old_pos.y = self.pos.y + vel.y * bounce
    end
end

function VerletPoint:constrain_to_ground(ground_y, bounce, friction)
    bounce = bounce or 0.3
    friction = friction or 0.95

    if self.pos.y > ground_y then
        self.pos.y = ground_y
        local vel = self:get_velocity()
        self.old_pos.y = self.pos.y + vel.y * bounce
        self.old_pos.x = self.pos.x - vel.x * friction
    end
end

-- Optional LÖVE drawing (only if love.graphics exists)
function VerletPoint:draw(color)
    if not love or not love.graphics then return end
    if color then love.graphics.setColor(color) else love.graphics.setColor(1, 1, 1) end
    love.graphics.circle('fill', self.pos.x, self.pos.y, self.radius)
end

function VerletPoint:draw_debug()
    if not love or not love.graphics then return end
    love.graphics.setColor(1, 1, 1)
    love.graphics.circle('line', self.pos.x, self.pos.y, self.radius)
    local vel = self:get_velocity()
    love.graphics.setColor(0, 1, 0, 0.7)
    love.graphics.line(self.pos.x, self.pos.y, self.pos.x + vel.x * 5, self.pos.y + vel.y * 5)
    if self.pinned then
        love.graphics.setColor(1, 0, 0)
        love.graphics.circle('fill', self.pos.x, self.pos.y, 3)
    end
end

--------------------------------------------------------------------------------
-- VerletChain: A chain of connected points with distance constraints
--------------------------------------------------------------------------------

local VerletChain = {}
VerletChain.__index = VerletChain

function VerletChain.new(start_x, start_y, segment_count, segment_length, options)
    options = options or {}
    local self = setmetatable({}, VerletChain)

    self.points = {}
    self.segment_length = segment_length
    self.constraint_iterations = options.constraint_iterations or 5

    for i = 1, segment_count do
        local x = start_x
        local y = start_y + (i - 1) * segment_length
        local point = VerletPoint.new(x, y, {
            friction = options.friction or 0.98,
            radius = options.radius or 5
        })
        table.insert(self.points, point)
    end

    return self
end

function VerletChain:solve_constraint(point_a, point_b, target_length)
    local delta = vec2.sub(point_b.pos, point_a.pos)
    local current_length = vec2.length(delta)

    if current_length == 0 then return end

    local diff = (target_length - current_length) / current_length
    local offset = vec2.mul(delta, 0.5 * diff)

    if not point_a.pinned then
        point_a.pos = vec2.sub(point_a.pos, offset)
    end
    if not point_b.pinned then
        point_b.pos = vec2.add(point_b.pos, offset)
    end
end

function VerletChain:update(dt, gravity)
    for _, point in ipairs(self.points) do
        point:update(dt, gravity)
    end

    for _ = 1, self.constraint_iterations do
        for i = 1, #self.points - 1 do
            self:solve_constraint(self.points[i], self.points[i + 1], self.segment_length)
        end
    end
end

function VerletChain:constrain_to_ground(ground_y, bounce, friction)
    for _, point in ipairs(self.points) do
        point:constrain_to_ground(ground_y, bounce, friction)
    end
end

function VerletChain:constrain_to_bounds(min_x, min_y, max_x, max_y, bounce)
    for _, point in ipairs(self.points) do
        point:constrain_to_bounds(min_x, min_y, max_x, max_y, bounce)
    end
end

function VerletChain:get_head() return self.points[1] end
function VerletChain:get_tail() return self.points[#self.points] end

-- Catmull-Rom spline interpolation
function VerletChain:catmull_rom(p0, p1, p2, p3, t)
    local t2, t3 = t * t, t * t * t
    local x = 0.5 * ((2 * p1.x) + (-p0.x + p2.x) * t +
              (2 * p0.x - 5 * p1.x + 4 * p2.x - p3.x) * t2 +
              (-p0.x + 3 * p1.x - 3 * p2.x + p3.x) * t3)
    local y = 0.5 * ((2 * p1.y) + (-p0.y + p2.y) * t +
              (2 * p0.y - 5 * p1.y + 4 * p2.y - p3.y) * t2 +
              (-p0.y + 3 * p1.y - 3 * p2.y + p3.y) * t3)
    return x, y
end

-- Drawing functions (require LÖVE)
function VerletChain:draw_lines(color)
    if not love or not love.graphics then return end
    if color then love.graphics.setColor(color) else love.graphics.setColor(1, 1, 1) end
    for i = 1, #self.points - 1 do
        local p1, p2 = self.points[i], self.points[i + 1]
        love.graphics.line(p1.pos.x, p1.pos.y, p2.pos.x, p2.pos.y)
    end
end

function VerletChain:draw_circles(sizes, color)
    if not love or not love.graphics then return end
    if color then love.graphics.setColor(color) else love.graphics.setColor(1, 1, 1) end
    for i, point in ipairs(self.points) do
        local radius = sizes and sizes[i] or point.radius
        love.graphics.circle('fill', point.pos.x, point.pos.y, radius)
    end
end

function VerletChain:draw_smooth(thickness, color)
    if not love or not love.graphics then return end
    if color then love.graphics.setColor(color) else love.graphics.setColor(1, 1, 1) end
    love.graphics.setLineWidth(thickness or 2)

    local curve_points = {}
    local steps = 10

    for i = 1, #self.points - 1 do
        local p0 = self.points[math.max(1, i - 1)]
        local p1 = self.points[i]
        local p2 = self.points[math.min(#self.points, i + 1)]
        local p3 = self.points[math.min(#self.points, i + 2)]

        for t = 0, 1, 1 / steps do
            local x, y = self:catmull_rom(p0.pos, p1.pos, p2.pos, p3.pos, t)
            table.insert(curve_points, x)
            table.insert(curve_points, y)
        end
    end

    if #curve_points >= 4 then
        love.graphics.line(curve_points)
    end
    love.graphics.setLineWidth(1)
end

function VerletChain:draw_debug()
    if not love or not love.graphics then return end
    love.graphics.setColor(0.5, 0.5, 0.5, 0.5)
    for i = 1, #self.points - 1 do
        local p1, p2 = self.points[i], self.points[i + 1]
        love.graphics.line(p1.pos.x, p1.pos.y, p2.pos.x, p2.pos.y)
    end
    for i, point in ipairs(self.points) do
        point:draw_debug()
        love.graphics.setColor(1, 1, 0)
        love.graphics.print(i, point.pos.x + 8, point.pos.y - 8)
    end
end

--------------------------------------------------------------------------------
-- Module exports
--------------------------------------------------------------------------------

Verlet.Point = VerletPoint.new
Verlet.Chain = VerletChain.new
Verlet.VerletPoint = VerletPoint
Verlet.VerletChain = VerletChain

return Verlet

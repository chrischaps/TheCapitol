-- 2D Vector Math Library
local vec2 = {}
vec2.__index = vec2

function vec2.new(x, y)
    return setmetatable({ x = x or 0, y = y or 0 }, vec2)
end

function vec2.clone(v)
    return vec2.new(v.x, v.y)
end

function vec2.add(a, b)
    return vec2.new(a.x + b.x, a.y + b.y)
end

function vec2.sub(a, b)
    return vec2.new(a.x - b.x, a.y - b.y)
end

function vec2.mul(v, scalar)
    return vec2.new(v.x * scalar, v.y * scalar)
end

function vec2.div(v, scalar)
    return vec2.new(v.x / scalar, v.y / scalar)
end

function vec2.length(v)
    return math.sqrt(v.x * v.x + v.y * v.y)
end

function vec2.length_squared(v)
    return v.x * v.x + v.y * v.y
end

function vec2.distance(a, b)
    local dx = b.x - a.x
    local dy = b.y - a.y
    return math.sqrt(dx * dx + dy * dy)
end

function vec2.normalize(v)
    local len = vec2.length(v)
    if len > 0 then
        return vec2.new(v.x / len, v.y / len)
    end
    return vec2.new(0, 0)
end

function vec2.dot(a, b)
    return a.x * b.x + a.y * b.y
end

function vec2.cross(a, b)
    return a.x * b.y - a.y * b.x
end

function vec2.lerp(a, b, t)
    return vec2.new(
        a.x + (b.x - a.x) * t,
        a.y + (b.y - a.y) * t
    )
end

function vec2.angle(v)
    return math.atan2(v.y, v.x)
end

function vec2.from_angle(angle, length)
    length = length or 1
    return vec2.new(math.cos(angle) * length, math.sin(angle) * length)
end

function vec2.rotate(v, angle)
    local c = math.cos(angle)
    local s = math.sin(angle)
    return vec2.new(v.x * c - v.y * s, v.x * s + v.y * c)
end

function vec2.perpendicular(v)
    return vec2.new(-v.y, v.x)
end

function vec2.reflect(v, normal)
    local d = 2 * vec2.dot(v, normal)
    return vec2.sub(v, vec2.mul(normal, d))
end

function vec2.clamp_length(v, max_length)
    local len = vec2.length(v)
    if len > max_length then
        return vec2.mul(vec2.normalize(v), max_length)
    end
    return vec2.clone(v)
end

-- Metamethods for convenience
function vec2.__add(a, b) return vec2.add(a, b) end
function vec2.__sub(a, b) return vec2.sub(a, b) end
function vec2.__mul(a, b)
    if type(a) == "number" then return vec2.mul(b, a) end
    if type(b) == "number" then return vec2.mul(a, b) end
    return vec2.dot(a, b)
end
function vec2.__unm(v) return vec2.new(-v.x, -v.y) end
function vec2.__tostring(v) return string.format("vec2(%.2f, %.2f)", v.x, v.y) end

return vec2

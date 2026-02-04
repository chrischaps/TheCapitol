-- Terrain rendering (water rings, bridges, roads)
local config = require("src.config")

local terrain = {}

-- Stored terrain data (fetched from server)
terrain.data = nil

-- Set terrain data from server response
function terrain.setData(data)
    terrain.data = data
end

-- Check if terrain data is loaded
function terrain.isLoaded()
    return terrain.data ~= nil
end

-- Get terrain data
function terrain.getData()
    return terrain.data
end

-- Draw all water rings
function terrain.drawWaterRings(offsetX, offsetY)
    if not terrain.data or not terrain.data.water_rings then return end

    local colors = config.COLORS
    local centerX = terrain.data.center_x + offsetX
    local centerY = terrain.data.center_y + offsetY

    for _, ring in ipairs(terrain.data.water_rings) do
        terrain.drawWaterRing(centerX, centerY, ring)
    end
end

-- Draw a single water ring (annulus) with gaps for bridges
function terrain.drawWaterRing(centerX, centerY, ring)
    local colors = config.COLORS
    local innerRadius = ring.radius - ring.width / 2
    local outerRadius = ring.radius + ring.width / 2

    -- Calculate bridge angular spans
    local bridgeSpans = {}
    for _, bridge in ipairs(ring.bridges) do
        -- Angular half-width in radians
        local halfWidthRad = (bridge.width / ring.radius)
        local centerRad = math.rad(bridge.angle_degrees)
        table.insert(bridgeSpans, {
            start = centerRad - halfWidthRad,
            finish = centerRad + halfWidthRad,
        })
    end

    -- Sort bridges by start angle
    table.sort(bridgeSpans, function(a, b) return a.start < b.start end)

    -- Draw water segments between bridges
    local segments = 64  -- Number of segments for the full circle
    local segmentAngle = math.pi * 2 / segments

    love.graphics.setColor(colors.WATER)

    for i = 0, segments - 1 do
        local angle1 = i * segmentAngle
        local angle2 = (i + 1) * segmentAngle

        -- Check if this segment overlaps with any bridge
        local onBridge = false
        for _, span in ipairs(bridgeSpans) do
            -- Normalize angles for comparison
            local midAngle = (angle1 + angle2) / 2
            local spanStart = span.start
            local spanFinish = span.finish

            -- Handle wraparound
            if spanStart < 0 then spanStart = spanStart + math.pi * 2 end
            if spanFinish < 0 then spanFinish = spanFinish + math.pi * 2 end
            if spanFinish < spanStart then
                -- Span crosses 0
                if midAngle >= spanStart or midAngle <= spanFinish then
                    onBridge = true
                    break
                end
            else
                if midAngle >= spanStart and midAngle <= spanFinish then
                    onBridge = true
                    break
                end
            end

            -- Also check original span (negative angles)
            if span.start < 0 and midAngle > math.pi then
                local normalizedMid = midAngle - math.pi * 2
                if normalizedMid >= span.start and normalizedMid <= span.finish then
                    onBridge = true
                    break
                end
            end
        end

        if not onBridge then
            -- Draw this water segment
            local x1Inner = centerX + math.cos(angle1) * innerRadius
            local y1Inner = centerY + math.sin(angle1) * innerRadius
            local x2Inner = centerX + math.cos(angle2) * innerRadius
            local y2Inner = centerY + math.sin(angle2) * innerRadius
            local x1Outer = centerX + math.cos(angle1) * outerRadius
            local y1Outer = centerY + math.sin(angle1) * outerRadius
            local x2Outer = centerX + math.cos(angle2) * outerRadius
            local y2Outer = centerY + math.sin(angle2) * outerRadius

            -- Draw as a quad (two triangles)
            love.graphics.polygon("fill",
                x1Inner, y1Inner,
                x2Inner, y2Inner,
                x2Outer, y2Outer,
                x1Outer, y1Outer
            )
        end
    end

    -- Draw water highlight/shimmer effect
    love.graphics.setColor(colors.WATER_HIGHLIGHT)
    love.graphics.setLineWidth(1)
    love.graphics.circle("line", centerX, centerY, ring.radius)
end

-- Draw all bridges
function terrain.drawBridges(offsetX, offsetY)
    if not terrain.data or not terrain.data.water_rings then return end

    local colors = config.COLORS
    local centerX = terrain.data.center_x + offsetX
    local centerY = terrain.data.center_y + offsetY

    for _, ring in ipairs(terrain.data.water_rings) do
        local innerRadius = ring.radius - ring.width / 2 - 2
        local outerRadius = ring.radius + ring.width / 2 + 2

        for _, bridge in ipairs(ring.bridges) do
            terrain.drawBridge(centerX, centerY, innerRadius, outerRadius, bridge)
        end
    end
end

-- Draw a single bridge
function terrain.drawBridge(centerX, centerY, innerRadius, outerRadius, bridge)
    local colors = config.COLORS
    local angleRad = math.rad(bridge.angle_degrees)
    local halfWidthRad = (bridge.width / innerRadius) * 1.2  -- Slightly wider for visual

    local angle1 = angleRad - halfWidthRad
    local angle2 = angleRad + halfWidthRad

    -- Calculate bridge corners
    local x1Inner = centerX + math.cos(angle1) * innerRadius
    local y1Inner = centerY + math.sin(angle1) * innerRadius
    local x2Inner = centerX + math.cos(angle2) * innerRadius
    local y2Inner = centerY + math.sin(angle2) * innerRadius
    local x1Outer = centerX + math.cos(angle1) * outerRadius
    local y1Outer = centerY + math.sin(angle1) * outerRadius
    local x2Outer = centerX + math.cos(angle2) * outerRadius
    local y2Outer = centerY + math.sin(angle2) * outerRadius

    -- Draw bridge surface
    love.graphics.setColor(colors.BRIDGE)
    love.graphics.polygon("fill",
        x1Inner, y1Inner,
        x2Inner, y2Inner,
        x2Outer, y2Outer,
        x1Outer, y1Outer
    )

    -- Draw bridge edges
    love.graphics.setColor(colors.BRIDGE_EDGE)
    love.graphics.setLineWidth(2)
    love.graphics.line(x1Inner, y1Inner, x1Outer, y1Outer)
    love.graphics.line(x2Inner, y2Inner, x2Outer, y2Outer)
end

-- Draw all roads
function terrain.drawRoads(offsetX, offsetY)
    if not terrain.data or not terrain.data.roads then return end

    local colors = config.COLORS

    -- Draw roads by type (trails first, then others on top)
    local roadsByType = {
        trail = {},
        suburban = {},
        grid = {},
        arterial = {},
    }

    for _, road in ipairs(terrain.data.roads) do
        local roadType = road.road_type or "trail"
        if roadsByType[roadType] then
            table.insert(roadsByType[roadType], road)
        end
    end

    -- Draw in order: trails (bottom) -> suburban -> grid -> arterial (top)
    terrain.drawRoadSet(roadsByType.trail, colors.ROAD_TRAIL, offsetX, offsetY)
    terrain.drawRoadSet(roadsByType.suburban, colors.ROAD_SUBURBAN, offsetX, offsetY)
    terrain.drawRoadSet(roadsByType.grid, colors.ROAD_GRID, offsetX, offsetY)
    terrain.drawRoadSet(roadsByType.arterial, colors.ROAD_ARTERIAL, offsetX, offsetY)
end

-- Draw a set of roads with a specific color
function terrain.drawRoadSet(roads, color, offsetX, offsetY)
    love.graphics.setColor(color)

    for _, road in ipairs(roads) do
        if road.points and #road.points >= 2 then
            love.graphics.setLineWidth(road.width or 8)

            -- Convert points to screen coords
            local points = {}
            for _, pt in ipairs(road.points) do
                table.insert(points, pt[1] + offsetX)
                table.insert(points, pt[2] + offsetY)
            end

            -- Draw as connected line segments
            if #points >= 4 then
                love.graphics.line(points)
            end
        end
    end
end

-- Draw everything (convenience function)
function terrain.draw(offsetX, offsetY)
    if not terrain.isLoaded() then
        -- Debug: terrain not loaded
        return
    end

    local centerX = terrain.data.center_x + offsetX
    local centerY = terrain.data.center_y + offsetY

    -- Draw in order: roads -> water -> bridges
    -- (Roads first so water can cover road ends at waterfront)
    terrain.drawRoads(offsetX, offsetY)
    terrain.drawWaterRings(offsetX, offsetY)
    terrain.drawBridges(offsetX, offsetY)

    -- DEBUG: Draw a visible marker at the world center
    love.graphics.setColor(1, 0, 0, 0.8)
    love.graphics.circle("fill", centerX, centerY, 20)
    love.graphics.setColor(1, 1, 0, 1)
    love.graphics.setLineWidth(3)
    love.graphics.circle("line", centerX, centerY, 50)
    love.graphics.circle("line", centerX, centerY, 200)  -- Moat radius
    love.graphics.circle("line", centerX, centerY, 600)  -- Inner canal
    love.graphics.circle("line", centerX, centerY, 1000) -- Outer canal
end

return terrain

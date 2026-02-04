-- World rendering (grid, boundary, nodes, terrain)
local config = require("src.config")
local game = require("src.state.game")
local terrain = require("src.terrain")

local world = {}

function world.drawGrid(offsetX, offsetY, screenWidth, screenHeight)
    local colors = config.COLORS
    local gridSize = config.GRID_SIZE
    local worldSize = config.WORLD_SIZE

    love.graphics.setColor(colors.GRID)
    love.graphics.setLineWidth(1)

    -- Vertical lines
    for x = 0, worldSize, gridSize do
        local screenX = x + offsetX
        if screenX >= -1 and screenX <= screenWidth + 1 then
            love.graphics.line(screenX, math.max(0, offsetY), screenX, math.min(screenHeight, worldSize + offsetY))
        end
    end

    -- Horizontal lines
    for y = 0, worldSize, gridSize do
        local screenY = y + offsetY
        if screenY >= -1 and screenY <= screenHeight + 1 then
            love.graphics.line(math.max(0, offsetX), screenY, math.min(screenWidth, worldSize + offsetX), screenY)
        end
    end
end

function world.drawBoundary(offsetX, offsetY)
    local colors = config.COLORS
    local worldSize = config.WORLD_SIZE

    love.graphics.setColor(colors.BOUNDARY)
    love.graphics.setLineWidth(2)
    love.graphics.rectangle("line", offsetX, offsetY, worldSize, worldSize)
end

-- Draw a single grass tuft
local function drawGrassTuft(x, y, isAvailable, isHarvesting, isPendingTarget)
    local colors = config.COLORS
    local nodeRadius = config.NODE_RADIUS

    -- Select colors based on state
    local baseColor, tipColor, groundColor
    if isAvailable then
        baseColor = colors.NODE_AVAILABLE
        tipColor = colors.NODE_AVAILABLE_TIP
        groundColor = colors.NODE_AVAILABLE_BASE
    else
        baseColor = colors.NODE_DEPLETED
        tipColor = colors.NODE_DEPLETED_TIP
        groundColor = colors.NODE_DEPLETED_BASE
    end

    local highlightColor = nil
    if isHarvesting or isPendingTarget then
        highlightColor = colors.GOLDEN
    end

    -- Draw targeting ring for pending extraction
    if isPendingTarget then
        love.graphics.setColor(colors.GOLDEN)
        love.graphics.setLineWidth(2)
        love.graphics.circle("line", x, y, nodeRadius + 4)
    end

    -- Grass blades
    local blades = {
        { angle = -30, height = 14 },
        { angle = -10, height = 18 },
        { angle = 10, height = 16 },
        { angle = 30, height = 12 },
        { angle = 0, height = 20 },
    }

    for _, blade in ipairs(blades) do
        local radians = math.rad(blade.angle)
        local tipX = x + math.sin(radians) * blade.height
        local tipY = y - blade.height

        -- Draw blade as curved shape (simplified to triangle)
        love.graphics.setColor(baseColor)
        love.graphics.polygon("fill",
            x - 2, y,
            tipX, tipY,
            x + 2, y
        )

        -- Highlight tip
        love.graphics.setColor(highlightColor or tipColor)
        love.graphics.circle("fill", tipX, tipY, 2)
    end

    -- Base mound
    love.graphics.setColor(groundColor)
    love.graphics.ellipse("fill", x, y + 2, 8, 4)
end

function world.drawNodes(offsetX, offsetY)
    for id, node in pairs(game.nodes) do
        local screenX = node.x + offsetX
        local screenY = node.y + offsetY

        local isAvailable = node.state == "available"
        local isHarvesting = node.state == "harvesting"
        local isPendingTarget = game.pendingExtractionNodeId == id

        drawGrassTuft(screenX, screenY, isAvailable, isHarvesting, isPendingTarget)
    end
end

-- Draw dashed line to pending extraction target
function world.drawTargetLine(offsetX, offsetY)
    if not game.pendingExtractionNodeId then
        return
    end

    local player = game.getMyPlayer()
    local node = game.nodes[game.pendingExtractionNodeId]
    if not player or not node then
        return
    end

    local colors = config.COLORS
    local playerX = player.x + offsetX
    local playerY = player.y + offsetY
    local nodeX = node.x + offsetX
    local nodeY = node.y + offsetY

    -- Calculate line direction
    local dx = nodeX - playerX
    local dy = nodeY - playerY
    local dist = math.sqrt(dx * dx + dy * dy)
    if dist == 0 then return end

    -- Draw dashed line
    love.graphics.setColor(colors.TARGET_LINE)
    love.graphics.setLineWidth(2)

    local dashLength = 10
    local gapLength = 10
    local totalLength = dashLength + gapLength

    local steps = math.floor(dist / totalLength)
    for i = 0, steps do
        local startT = (i * totalLength) / dist
        local endT = math.min(1, (i * totalLength + dashLength) / dist)

        local x1 = playerX + dx * startT
        local y1 = playerY + dy * startT
        local x2 = playerX + dx * endT
        local y2 = playerY + dy * endT

        love.graphics.line(x1, y1, x2, y2)
    end
end

-- Draw terrain (water rings, bridges, roads)
function world.drawTerrain(offsetX, offsetY)
    terrain.draw(offsetX, offsetY)
end

return world

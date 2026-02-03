-- Inventory panel component
local config = require("src.config")
local game = require("src.state.game")
local ui = require("src.state.ui")
local Button = require("src.ui.button")

local inventory = {}

local panelWidth = 300
local panelHeight = 400
local itemHeight = 50
local scrollOffset = 0
local maxScrollOffset = 0

local toggleButton = nil

function inventory.init()
    toggleButton = Button.new(0, 0, 40, 40, "I", function()
        ui.toggleInventory()
    end)
end

function inventory.update(dt, screenWidth, screenHeight, mx, my)
    -- Position toggle button in top-right
    toggleButton:setPosition(screenWidth - 50, 10)
    toggleButton:update(mx, my)

    -- Calculate max scroll
    local contentHeight = #game.inventory * itemHeight
    maxScrollOffset = math.max(0, contentHeight - (panelHeight - 60))
end

function inventory.mousepressed(x, y, btn, screenWidth, screenHeight)
    if toggleButton:mousepressed(x, y, btn) then
        return true
    end

    -- Check if clicking inside panel
    if ui.showInventory then
        local panelX = screenWidth - panelWidth - 10
        local panelY = 60
        if x >= panelX and x <= panelX + panelWidth
           and y >= panelY and y <= panelY + panelHeight then
            return true -- Consume click
        end
    end

    return false
end

function inventory.mousereleased(x, y, btn)
    toggleButton:mousereleased(x, y, btn)
end

function inventory.wheelmoved(x, y, screenWidth, screenHeight)
    if ui.showInventory then
        local mx, my = love.mouse.getPosition()
        local panelX = screenWidth - panelWidth - 10
        local panelY = 60
        if mx >= panelX and mx <= panelX + panelWidth
           and my >= panelY and my <= panelY + panelHeight then
            scrollOffset = scrollOffset - y * 30
            scrollOffset = math.max(0, math.min(maxScrollOffset, scrollOffset))
            return true
        end
    end
    return false
end

function inventory.keypressed(key)
    if key == "i" then
        ui.toggleInventory()
        return true
    end
    return false
end

local function getQualityColor(grade)
    local colors = config.COLORS
    if grade == "A" then return colors.QUALITY_A
    elseif grade == "B" then return colors.QUALITY_B
    elseif grade == "C" then return colors.QUALITY_C
    elseif grade == "D" then return colors.QUALITY_D
    else return colors.QUALITY_F
    end
end

function inventory.draw(screenWidth, screenHeight)
    local colors = config.COLORS

    -- Draw toggle button with item count badge
    toggleButton:draw()

    if #game.inventory > 0 then
        local badgeX = toggleButton.x + toggleButton.width - 8
        local badgeY = toggleButton.y - 4
        love.graphics.setColor(colors.GOLDEN)
        love.graphics.circle("fill", badgeX, badgeY, 10)
        love.graphics.setColor(colors.BACKGROUND)
        local countStr = tostring(#game.inventory)
        local font = love.graphics.getFont()
        love.graphics.print(countStr, badgeX - font:getWidth(countStr) / 2, badgeY - font:getHeight() / 2)
    end

    -- Draw panel if open
    if not ui.showInventory then
        return
    end

    local panelX = screenWidth - panelWidth - 10
    local panelY = 60

    -- Panel background
    love.graphics.setColor(colors.PANEL_BG)
    love.graphics.rectangle("fill", panelX, panelY, panelWidth, panelHeight, 8, 8)

    -- Panel border
    love.graphics.setColor(colors.PANEL_BORDER)
    love.graphics.setLineWidth(2)
    love.graphics.rectangle("line", panelX, panelY, panelWidth, panelHeight, 8, 8)

    -- Header
    love.graphics.setColor(colors.GOLDEN)
    love.graphics.print("Inventory", panelX + 15, panelY + 15)

    -- Items (with scrolling)
    local itemsY = panelY + 50
    local itemsHeight = panelHeight - 60

    love.graphics.setScissor(panelX, itemsY, panelWidth, itemsHeight)

    for i, item in ipairs(game.inventory) do
        local itemY = itemsY + (i - 1) * itemHeight - scrollOffset

        -- Skip if not visible
        if itemY + itemHeight > itemsY and itemY < itemsY + itemsHeight then
            -- Item background (alternating)
            if i % 2 == 0 then
                love.graphics.setColor(0.05, 0.05, 0.07, 0.5)
                love.graphics.rectangle("fill", panelX + 10, itemY, panelWidth - 20, itemHeight - 5, 4, 4)
            end

            -- Item name
            love.graphics.setColor(colors.TEXT)
            love.graphics.print(item.name, panelX + 15, itemY + 5)

            -- Quantity
            love.graphics.setColor(colors.TEXT_DIM)
            love.graphics.print("x" .. item.quantity, panelX + 15, itemY + 25)

            -- Quality badge
            local qualityColor = getQualityColor(item.quality_grade)
            local badgeX = panelX + panelWidth - 40
            local badgeY = itemY + 10

            love.graphics.setColor(qualityColor)
            love.graphics.rectangle("fill", badgeX, badgeY, 25, 25, 4, 4)

            love.graphics.setColor(colors.BACKGROUND)
            local font = love.graphics.getFont()
            local grade = item.quality_grade
            love.graphics.print(grade, badgeX + (25 - font:getWidth(grade)) / 2, badgeY + (25 - font:getHeight()) / 2)
        end
    end

    love.graphics.setScissor()

    -- Scroll indicator
    if maxScrollOffset > 0 then
        local scrollbarHeight = itemsHeight * (itemsHeight / (itemsHeight + maxScrollOffset))
        local scrollbarY = itemsY + (scrollOffset / maxScrollOffset) * (itemsHeight - scrollbarHeight)

        love.graphics.setColor(colors.PANEL_BORDER)
        love.graphics.rectangle("fill", panelX + panelWidth - 8, scrollbarY, 4, scrollbarHeight, 2, 2)
    end

    -- Empty state
    if #game.inventory == 0 then
        love.graphics.setColor(colors.TEXT_DIM)
        love.graphics.printf("No items yet", panelX, panelY + panelHeight / 2 - 10, panelWidth, "center")
    end
end

return inventory

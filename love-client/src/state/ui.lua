-- UI state management
local ui = {}

-- Panel visibility
ui.showInventory = false

-- Connection status
ui.connectionStatus = "disconnected" -- "disconnected", "connecting", "connected"
ui.connectionError = nil

-- Loading state
ui.isLoading = false
ui.loadingMessage = ""

-- Toggle inventory panel
function ui.toggleInventory()
    ui.showInventory = not ui.showInventory
end

-- Set connection status
function ui.setConnecting()
    ui.connectionStatus = "connecting"
    ui.connectionError = nil
end

function ui.setConnected()
    ui.connectionStatus = "connected"
    ui.connectionError = nil
end

function ui.setDisconnected(error)
    ui.connectionStatus = "disconnected"
    ui.connectionError = error
end

-- Loading state
function ui.setLoading(message)
    ui.isLoading = true
    ui.loadingMessage = message or "Loading..."
end

function ui.clearLoading()
    ui.isLoading = false
    ui.loadingMessage = ""
end

return ui

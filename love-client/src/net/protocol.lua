-- WebSocket protocol message types
local protocol = {}

-- Client -> Server message types
protocol.CLIENT = {
    AUTH = "auth",
    SUBSCRIBE = "subscribe",
    MOVE = "move",
    EXTRACT = "extract",
    CANCEL_EXTRACTION = "cancel_extraction",
}

-- Server -> Client message types
protocol.SERVER = {
    AUTH_OK = "auth_ok",
    AUTH_ERROR = "auth_error",
    SUBSCRIBED = "subscribed",
    POSITIONS = "positions",
    PLAYER_JOINED = "player_joined",
    PLAYER_LEFT = "player_left",
    ERROR = "error",
    EXTRACTION_STARTED = "extraction_started",
    EXTRACTION_PROGRESS = "extraction_progress",
    EXTRACTION_COMPLETED = "extraction_completed",
    EXTRACTION_CANCELLED = "extraction_cancelled",
    NODE_DEPLETED = "node_depleted",
    NODE_REGENERATED = "node_regenerated",
    NEARBY_NODES = "nearby_nodes",
    -- Terrain events
    MOVEMENT_BLOCKED = "movement_blocked",
    ZONE_CHANGED = "zone_changed",
}

-- Subscription channels
protocol.CHANNELS = {
    POSITION = "position",
}

return protocol

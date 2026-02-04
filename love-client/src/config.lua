-- Configuration constants for The Capitol client
local config = {}

-- World dimensions
config.WORLD_SIZE = 8000
config.WORLD_CENTER = 4000
config.GRID_SIZE = 100

-- Entity sizes
config.PLAYER_RADIUS = 8
config.NODE_RADIUS = 12

-- Gameplay
config.EXTRACTION_RANGE = 50
config.MOVE_SPEED = 100 -- units per second (for prediction)
config.NODE_REFRESH_INTERVAL = 5 -- seconds

-- Colors (RGBA 0-1)
config.COLORS = {
    GOLDEN = {0.788, 0.635, 0.153, 1},         -- #c9a227
    BACKGROUND = {0.039, 0.039, 0.059, 1},     -- #0a0a0f
    GRID = {0.102, 0.102, 0.141, 1},           -- #1a1a24
    BOUNDARY = {0.788, 0.635, 0.153, 1},       -- #c9a227

    -- Players
    PLAYER_SELF = {0.788, 0.635, 0.153, 1},    -- #c9a227
    PLAYER_SELF_STROKE = {1, 1, 1, 1},         -- #ffffff
    PLAYER_OTHER = {0.290, 0.486, 0.349, 1},   -- #4a7c59
    PLAYER_OTHER_STROKE = {0.176, 0.306, 0.212, 1}, -- #2d4e36

    -- Nodes
    NODE_AVAILABLE = {0.290, 0.486, 0.349, 1}, -- #4a7c59
    NODE_AVAILABLE_TIP = {0.420, 0.639, 0.467, 1}, -- #6ba377
    NODE_AVAILABLE_BASE = {0.239, 0.369, 0.259, 1}, -- #3d5e42
    NODE_DEPLETED = {0.227, 0.227, 0.267, 1},  -- #3a3a44
    NODE_DEPLETED_TIP = {0.290, 0.290, 0.329, 1}, -- #4a4a54
    NODE_DEPLETED_BASE = {0.165, 0.165, 0.204, 1}, -- #2a2a34

    -- UI
    TEXT = {0.878, 0.878, 0.878, 1},           -- #e0e0e0
    TEXT_DIM = {0.6, 0.6, 0.6, 1},
    PANEL_BG = {0.059, 0.059, 0.078, 0.95},    -- #0f0f14
    PANEL_BORDER = {0.157, 0.157, 0.196, 1},   -- #282832
    INPUT_BG = {0.078, 0.078, 0.098, 1},       -- #141419

    -- Quality grades
    QUALITY_A = {0.290, 0.686, 0.349, 1},      -- Green
    QUALITY_B = {0.290, 0.486, 0.686, 1},      -- Blue
    QUALITY_C = {0.788, 0.635, 0.153, 1},      -- Gold
    QUALITY_D = {0.788, 0.486, 0.153, 1},      -- Orange
    QUALITY_F = {0.686, 0.290, 0.290, 1},      -- Red

    -- Extraction
    EXTRACTION_RING_BG = {0.788, 0.635, 0.153, 0.3},
    EXTRACTION_RING = {0.788, 0.635, 0.153, 1},
    TARGET_LINE = {0.788, 0.635, 0.153, 0.5},

    -- Terrain: Water
    WATER = {0.15, 0.25, 0.45, 0.9},           -- Deep blue
    WATER_HIGHLIGHT = {0.2, 0.35, 0.55, 0.7},  -- Lighter blue shimmer

    -- Terrain: Bridges
    BRIDGE = {0.35, 0.30, 0.25, 1.0},          -- Dark brown wood
    BRIDGE_EDGE = {0.25, 0.20, 0.15, 1.0},     -- Darker edge

    -- Terrain: Roads
    ROAD_ARTERIAL = {0.25, 0.22, 0.20, 1.0},   -- Dark stone
    ROAD_GRID = {0.30, 0.28, 0.25, 1.0},       -- Medium stone
    ROAD_SUBURBAN = {0.35, 0.32, 0.28, 1.0},   -- Light stone/packed dirt
    ROAD_TRAIL = {0.40, 0.35, 0.25, 0.7},      -- Unpaved dirt path
}

-- Server URLs
config.API_URL = "http://localhost:3000"
config.WS_HOST = "localhost"
config.WS_PORT = 3000
config.WS_PATH = "/ws"

-- Virtual resolution for scaling
config.VIRTUAL_WIDTH = 1280
config.VIRTUAL_HEIGHT = 720

return config

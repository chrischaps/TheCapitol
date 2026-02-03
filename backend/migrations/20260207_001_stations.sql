-- M4: Crafting Stations & Storage Chests

-- Station type definitions
CREATE TABLE station_types (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    category VARCHAR(50) NOT NULL DEFAULT 'crafting',  -- 'crafting' or 'storage'
    slot_count INTEGER NOT NULL DEFAULT 16,
    layout_columns INTEGER NOT NULL DEFAULT 4,
    interaction_range REAL NOT NULL DEFAULT 75.0,
    icon VARCHAR(50),
    description TEXT
);

-- Station instances in world
CREATE TABLE stations (
    id UUID PRIMARY KEY,
    station_type VARCHAR(50) NOT NULL REFERENCES station_types(id),
    owner_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    position_x REAL NOT NULL,
    position_y REAL NOT NULL,
    placed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_stations_position ON stations(position_x, position_y);
CREATE INDEX idx_stations_owner ON stations(owner_id);

-- Add station requirement to recipes
ALTER TABLE recipes ADD COLUMN required_station_type VARCHAR(50)
    REFERENCES station_types(id);

-- New container type for stations
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('station_inventory', 'Station Inventory', 16, 4);

-- Seed station types
INSERT INTO station_types (id, name, category, slot_count, layout_columns, icon, description) VALUES
    ('workbench', 'Workbench', 'crafting', 16, 4, 'workbench', 'Craft wooden items and tools'),
    ('forge', 'Forge', 'crafting', 12, 4, 'forge', 'Smelt ores and craft metal equipment'),
    ('storage_chest', 'Storage Chest', 'storage', 16, 4, 'chest', 'Store items safely');

-- Placeable station kit items
INSERT INTO item_types (id, name, category, stackable, weight) VALUES
    ('workbench_kit', 'Workbench Kit', 'placeable', false, 10),
    ('forge_kit', 'Forge Kit', 'placeable', false, 15),
    ('chest_kit', 'Storage Chest Kit', 'placeable', false, 8);

-- Recipes to craft station kits (hand-craftable)
INSERT INTO recipes (id, name, description, category, inputs, outputs, base_duration_ticks, quality_weights, crafting_requirements)
VALUES
    ('craft_workbench', 'Workbench Kit', 'Craft a portable workbench kit.', 'hand_crafting',
     '[{"item_type": "fiber", "quantity": 10, "quality_weight": 1.0}]',
     '[{"item_type": "workbench_kit", "quantity_min": 1, "quantity_max": 1}]',
     100,
     '{"variance_min": -3, "variance_max": 3}',
     '{}'),
    ('craft_chest', 'Storage Chest Kit', 'Craft a portable storage chest kit.', 'hand_crafting',
     '[{"item_type": "fiber", "quantity": 8, "quality_weight": 1.0}]',
     '[{"item_type": "chest_kit", "quantity_min": 1, "quantity_max": 1}]',
     80,
     '{"variance_min": -3, "variance_max": 3}',
     '{}');

-- Add foreign key from containers to stations (now that stations table exists)
ALTER TABLE containers ADD CONSTRAINT fk_containers_station
    FOREIGN KEY (station_id) REFERENCES stations(id) ON DELETE CASCADE;

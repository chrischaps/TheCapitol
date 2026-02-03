-- M2: Gathering - Resource types, items, and resource nodes

-- Resource Types (static data)
CREATE TABLE resource_types (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    yields_item VARCHAR(50) NOT NULL,
    yield_quantity_min INTEGER NOT NULL DEFAULT 1,
    yield_quantity_max INTEGER NOT NULL DEFAULT 3,
    base_quality_min INTEGER NOT NULL DEFAULT 40,
    base_quality_max INTEGER NOT NULL DEFAULT 70,
    required_tool VARCHAR(50),  -- NULL = hand-gatherable
    extraction_duration_ticks INTEGER NOT NULL DEFAULT 30,  -- 3 seconds at 10 ticks/sec
    regeneration_ticks INTEGER NOT NULL DEFAULT 600  -- 60 seconds
);

-- Item Types (static data)
CREATE TABLE item_types (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    category VARCHAR(50) NOT NULL,
    stackable BOOLEAN NOT NULL DEFAULT true,
    weight REAL NOT NULL DEFAULT 0.1
);

-- Resource Nodes (world instances)
CREATE TABLE resource_nodes (
    id UUID PRIMARY KEY,
    resource_type VARCHAR(50) NOT NULL REFERENCES resource_types(id),
    position_x DOUBLE PRECISION NOT NULL,
    position_y DOUBLE PRECISION NOT NULL,
    base_quality INTEGER NOT NULL CHECK (base_quality BETWEEN 0 AND 100),
    state VARCHAR(20) NOT NULL DEFAULT 'available',
    harvested_by UUID REFERENCES players(id),
    regenerates_at_tick BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Items (player inventory)
CREATE TABLE items (
    id UUID PRIMARY KEY,
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    quality INTEGER NOT NULL CHECK (quality BETWEEN 0 AND 100),
    quantity INTEGER NOT NULL DEFAULT 1,
    owner_id UUID REFERENCES players(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Player action state columns
ALTER TABLE players ADD COLUMN action_state VARCHAR(20) DEFAULT 'idle';
ALTER TABLE players ADD COLUMN action_target_id UUID;
ALTER TABLE players ADD COLUMN action_progress INTEGER DEFAULT 0;

-- Seed data: resource types
INSERT INTO resource_types (id, name, yields_item, yield_quantity_min, yield_quantity_max, base_quality_min, base_quality_max, required_tool, extraction_duration_ticks, regeneration_ticks)
VALUES ('grass', 'Wild Grass', 'fiber', 1, 3, 40, 70, NULL, 30, 600);

-- Seed data: item types
INSERT INTO item_types (id, name, category, stackable, weight)
VALUES ('fiber', 'Plant Fiber', 'raw_material', true, 0.1);

-- Seed 50 grass nodes across the world in an 8x7 grid pattern with some randomness
INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality, state)
SELECT
    gen_random_uuid(),
    'grass',
    100 + ((n - 1) % 8) * 100 + (random() * 50 - 25),
    100 + ((n - 1) / 8) * 100 + (random() * 50 - 25),
    40 + floor(random() * 31)::int,
    'available'
FROM generate_series(1, 50) AS n;

-- Indexes for performance
CREATE INDEX idx_resource_nodes_state ON resource_nodes(state);
CREATE INDEX idx_resource_nodes_position ON resource_nodes(position_x, position_y);
CREATE INDEX idx_items_owner ON items(owner_id);

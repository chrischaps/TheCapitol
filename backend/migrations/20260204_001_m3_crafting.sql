-- M3: Crafting - Recipe system and quality cascade

-- Recipes table (static data)
CREATE TABLE recipes (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    description TEXT,
    category VARCHAR(50) NOT NULL DEFAULT 'hand_crafting',
    inputs JSONB NOT NULL,
    outputs JSONB NOT NULL,
    base_duration_ticks INTEGER NOT NULL DEFAULT 50,
    quality_weights JSONB NOT NULL DEFAULT '{"variance_min": -3, "variance_max": 3}',
    crafting_requirements JSONB NOT NULL DEFAULT '{}'
);

-- Add rope item type
INSERT INTO item_types (id, name, category, stackable, weight)
VALUES ('rope', 'Rope', 'processed_material', true, 0.2);

-- Basic rope recipe: 2 fiber -> 1 rope
INSERT INTO recipes (id, name, description, category, inputs, outputs, base_duration_ticks, quality_weights, crafting_requirements)
VALUES (
    'rope_basic',
    'Basic Rope',
    'Twist plant fibers into sturdy rope.',
    'hand_crafting',
    '[{"item_type": "fiber", "quantity": 2, "quality_weight": 0.5}]',
    '[{"item_type": "rope", "quantity_min": 1, "quantity_max": 1}]',
    50,
    '{"variance_min": -3, "variance_max": 3}',
    '{}'
);

-- Index for recipe lookups
CREATE INDEX idx_recipes_category ON recipes(category);

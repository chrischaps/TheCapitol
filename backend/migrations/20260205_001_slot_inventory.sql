-- Slot-based inventory system with containers

-- Container types (player_inventory, crafting_input, crafting_output, future station inventories)
CREATE TABLE container_types (
    id VARCHAR(50) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    slot_count INTEGER NOT NULL,
    layout_columns INTEGER NOT NULL DEFAULT 6
);

-- Container instances
CREATE TABLE containers (
    id UUID PRIMARY KEY,
    container_type VARCHAR(50) NOT NULL REFERENCES container_types(id),
    owner_id UUID REFERENCES players(id) ON DELETE CASCADE,
    station_id UUID,  -- For M4 stations (future)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_containers_owner ON containers(owner_id);
CREATE INDEX idx_containers_station ON containers(station_id) WHERE station_id IS NOT NULL;

-- Modify items: add container reference and slot position
ALTER TABLE items ADD COLUMN container_id UUID REFERENCES containers(id) ON DELETE CASCADE;
ALTER TABLE items ADD COLUMN slot_index INTEGER;

-- Unique constraint: one item stack per slot
CREATE UNIQUE INDEX idx_items_container_slot ON items(container_id, slot_index)
    WHERE container_id IS NOT NULL AND slot_index IS NOT NULL;

-- Seed container types
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('player_inventory', 'Player Inventory', 24, 6),
    ('crafting_input', 'Crafting Input', 4, 2),
    ('crafting_output', 'Crafting Output', 1, 1);

-- Data migration: Create inventory container for each existing player
-- and assign their items to slots (merging same-type items)
DO $$
DECLARE
    player_rec RECORD;
    container_uuid UUID;
    item_rec RECORD;
    slot_num INTEGER;
    existing_item_id UUID;
    existing_qty INTEGER;
    existing_quality INTEGER;
    new_quality INTEGER;
    new_qty INTEGER;
BEGIN
    -- For each player
    FOR player_rec IN SELECT id FROM players LOOP
        -- Create player inventory container
        container_uuid := gen_random_uuid();
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (container_uuid, 'player_inventory', player_rec.id);

        slot_num := 0;

        -- Process each unique item type for this player
        FOR item_rec IN
            SELECT item_type,
                   SUM(quantity) as total_qty,
                   SUM(quality * quantity) as weighted_quality
            FROM items
            WHERE owner_id = player_rec.id
            GROUP BY item_type
        LOOP
            IF slot_num < 24 THEN
                -- Calculate weighted average quality
                new_quality := item_rec.weighted_quality / item_rec.total_qty;

                -- Update one item to be the merged stack, delete others
                SELECT id, quantity, quality INTO existing_item_id, existing_qty, existing_quality
                FROM items
                WHERE owner_id = player_rec.id AND item_type = item_rec.item_type
                LIMIT 1;

                -- Update the kept item with merged values and container assignment
                UPDATE items
                SET container_id = container_uuid,
                    slot_index = slot_num,
                    quantity = item_rec.total_qty,
                    quality = new_quality
                WHERE id = existing_item_id;

                -- Delete the other items of same type
                DELETE FROM items
                WHERE owner_id = player_rec.id
                  AND item_type = item_rec.item_type
                  AND id != existing_item_id;

                slot_num := slot_num + 1;
            END IF;
        END LOOP;

        -- Create crafting input container for player
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (gen_random_uuid(), 'crafting_input', player_rec.id);

        -- Create crafting output container for player
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (gen_random_uuid(), 'crafting_output', player_rec.id);
    END LOOP;
END $$;

-- Make owner_id nullable for container-based items (container owns them now)
-- Items can either have owner_id (legacy) or container_id (new slot-based)

-- Create missing containers for existing players who don't have them

DO $$
DECLARE
    player_rec RECORD;
BEGIN
    -- For each player missing a player_inventory container
    FOR player_rec IN
        SELECT p.id
        FROM players p
        WHERE NOT EXISTS (
            SELECT 1 FROM containers c
            WHERE c.owner_id = p.id AND c.container_type = 'player_inventory'
        )
    LOOP
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (gen_random_uuid(), 'player_inventory', player_rec.id);
    END LOOP;

    -- For each player missing a crafting_input container
    FOR player_rec IN
        SELECT p.id
        FROM players p
        WHERE NOT EXISTS (
            SELECT 1 FROM containers c
            WHERE c.owner_id = p.id AND c.container_type = 'crafting_input'
        )
    LOOP
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (gen_random_uuid(), 'crafting_input', player_rec.id);
    END LOOP;

    -- For each player missing a crafting_output container
    FOR player_rec IN
        SELECT p.id
        FROM players p
        WHERE NOT EXISTS (
            SELECT 1 FROM containers c
            WHERE c.owner_id = p.id AND c.container_type = 'crafting_output'
        )
    LOOP
        INSERT INTO containers (id, container_type, owner_id)
        VALUES (gen_random_uuid(), 'crafting_output', player_rec.id);
    END LOOP;
END $$;

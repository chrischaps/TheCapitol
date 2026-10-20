-- The impound (#141): where a foreclosure, a forced terrain commit or a lot
-- laid under a workbench puts what a player's inventory has no room for.
-- Things only come out of it (`container_access::impound_refusal`). Its 480
-- slots hold an estate's 25 stations with full boxes, their kits and tools.
-- The content sync keeps the type; it's here for databases that run only the
-- migrations (`sqlx::test`).
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('impound', 'Impound', 480, 8)
ON CONFLICT (id) DO NOTHING;

-- One impound a player, made the first time something goes into it
-- (engine::impound::impound).
CREATE UNIQUE INDEX idx_containers_one_impound ON containers (owner_id)
    WHERE container_type = 'impound';

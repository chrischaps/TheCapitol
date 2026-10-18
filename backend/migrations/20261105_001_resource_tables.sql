-- E2.2 (#161): resource types as D5 has them.
--
-- - Every resource has a difficulty now, on the tools' power scale (hands are
--   4), not only those a tool works. Grass and branches are 0: hands' work.
-- - yields: units per gather by 'hand' or tool tier, {min, max, ticks?}
--   (protocol rules::resources::ResourceYield). yield_quantity_min/max stay,
--   as the hand row (or the lowest tier's, for a resource that needs a tool).
-- - cluster and spawns: how a resource's nodes group, and the zones they grow
--   in with a density each (rules::resources::Spawn). E2.3 places from these.
--
-- The values come from backend/content/resources.toml, upserted at startup.
-- Grass is set here as well, so the seeded row matches what the sync writes.
ALTER TABLE resource_types
    DROP CONSTRAINT resource_types_tool_has_difficulty;

UPDATE resource_types SET difficulty = 0 WHERE difficulty IS NULL;

ALTER TABLE resource_types
    ALTER COLUMN difficulty SET DEFAULT 0,
    ALTER COLUMN difficulty SET NOT NULL,
    ADD CONSTRAINT resource_types_difficulty_not_negative CHECK (difficulty >= 0),
    ADD COLUMN yields JSONB NOT NULL DEFAULT '{}',
    ADD COLUMN cluster VARCHAR(20) NOT NULL DEFAULT 'scattered',
    ADD COLUMN spawns JSONB NOT NULL DEFAULT '[]';

UPDATE resource_types SET
    difficulty = 0.0,
    yields = '{"hand": {"min": 1, "max": 3}, "stone": {"min": 2, "max": 4}, "iron": {"min": 3, "max": 5}}',
    cluster = 'scattered',
    spawns = '[{"zone": "suburban", "density": 5.0}, {"zone": "rural", "density": 2.5}]'
WHERE id = 'grass';

-- E1.5 (#157): tools at work. A resource type may name the tool type that
-- works it, with a difficulty: the power that counts as the right tool for it
-- (protocol rules::tools::leverage). required_tool, unused until now, becomes
-- a tool type too: one it can't be gathered without, and so its tool_type.
-- The values come from backend/content/resources.toml, upserted at startup;
-- grass is set here as well, so the seeded row matches what the sync writes.
ALTER TABLE resource_types
    ADD COLUMN tool_type VARCHAR(20),
    ADD COLUMN difficulty DOUBLE PRECISION,
    ADD CONSTRAINT resource_types_tool_has_difficulty CHECK (
        (tool_type IS NULL AND difficulty IS NULL)
        OR (tool_type IS NOT NULL AND difficulty > 0)
    ),
    ADD CONSTRAINT resource_types_required_tool_works_it CHECK (
        required_tool IS NULL OR required_tool = tool_type
    );

UPDATE resource_types SET tool_type = 'sickle', difficulty = 6.0 WHERE id = 'grass';

-- Store world generation parameters so admin tweaks persist across restarts.
-- Single-row table: the CHECK constraint ensures only one config row ever exists.
CREATE TABLE world_config (
    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    params JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

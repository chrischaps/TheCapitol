-- Tunables (the Observatory's D6.1, #375): the economy's knobs an admin has
-- moved off their defaults. `key` is a `capitol_protocol::tunables::KNOBS`
-- key, or a family's `deed_price.<zone>` or `yield.<resource>`. A knob set back
-- to its default loses its row, so a default changed in code reaches every
-- world that never moved it. Written by `engine::tunables::update`, read at boot.
CREATE TABLE tunables (
    key TEXT PRIMARY KEY,
    value DOUBLE PRECISION NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

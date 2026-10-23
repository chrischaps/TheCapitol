-- Weather (Plots v2 §5.2, V3.2, #474): a station's condition, 0 to 100.
--
-- Indoors it doesn't wear. Outdoors the weather wears it, slowly on a claimed
-- plot and fast on public land, where it falls down at 0. Nothing ticks it:
-- `condition` is as it stood at `condition_at`, and the hourly sweep
-- (`engine::weather::sweep`) and a repair write it on from there.
ALTER TABLE stations
    ADD COLUMN condition DOUBLE PRECISION NOT NULL DEFAULT 100
        CHECK (condition >= 0 AND condition <= 100),
    ADD COLUMN condition_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- The sweep reads only the outdoor stations still standing
CREATE INDEX stations_weathering ON stations (id) WHERE NOT indoors AND condition > 0;

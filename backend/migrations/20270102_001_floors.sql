-- Floors: building upward (Plots v2 §5.3 and §5.4, V3.3, #475).
--
-- A home lot's building may have floors over the ground, each holding its
-- indoor room again, so capacity is indoor_room * (1 + floors) + yard_room
-- (`models::plot::STATION_CAPACITY`). The last floor goes up until
-- floor_ready_at, and nothing is placed on it before then. floor_permits is
-- what its owner paid in permits, which the assessed value includes and a
-- release refunds.
ALTER TABLE plots
    ADD COLUMN floors INT NOT NULL DEFAULT 0 CHECK (floors >= 0),
    ADD COLUMN floor_ready_at TIMESTAMPTZ,
    ADD COLUMN floor_permits BIGINT NOT NULL DEFAULT 0;

-- The storey a station stands on: 0 the ground, k the kth floor. Each storey
-- of a building is a place of its own, with its own room and its own spacing.
ALTER TABLE stations
    ADD COLUMN floor INT NOT NULL DEFAULT 0 CHECK (floor >= 0);

-- A fire (the kiln, the forge) stands on the ground floor only. The content
-- sync sets it from stations.toml; the rows the migrations seeded get it here,
-- so the sync leaves them as they are.
ALTER TABLE station_types
    ADD COLUMN ground_only BOOLEAN NOT NULL DEFAULT FALSE;
UPDATE station_types SET ground_only = TRUE WHERE id IN ('kiln', 'forge');

-- The Gazette prints new floors: a deed event of its own, with its storey
ALTER TABLE deed_events ADD COLUMN floor INT;
ALTER TABLE deed_events DROP CONSTRAINT deed_events_kind_check;
ALTER TABLE deed_events ADD CONSTRAINT deed_events_kind_check
    CHECK (kind IN ('claimed', 'foreclosed', 'granted', 'hall', 'floor'));

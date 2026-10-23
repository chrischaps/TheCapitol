-- The building that comes with a deed, and the room it makes (Plots v2 §5.1
-- and §5.6, V3.1, #473).
--
-- A home lot's capacity is its indoor room times (1 + floors) plus its yard
-- room, each packed at STATION_MIN_SPACING (`station_room::room`). Packing
-- isn't something SQL can do, so the columns start NULL and the startup sync
-- (`plot_sync::fill_rooms`) fills in every lot that has none, today's lots
-- among them, and marks which of their stations stand indoors. A lot that
-- changes shape is cleared and filled again. NULL is also a lot with no
-- building (a guild hall), whose capacity is still its size category's.
ALTER TABLE plots
    ADD COLUMN indoor_room INT,
    ADD COLUMN yard_room INT;

-- Inside its lot's building, set when it's placed. A wall stands between a
-- station indoors and one out, so the spacing holds within each, not across.
ALTER TABLE stations
    ADD COLUMN indoors BOOLEAN NOT NULL DEFAULT FALSE;

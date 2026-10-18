-- #187: a station type may stand off a plot, on dry ground no lot covers.
-- Only the workbench does. The value comes from backend/content/stations.toml,
-- upserted at startup; this row fills it for the station that has it today,
-- so a database is the same before and after its first content upsert.
ALTER TABLE station_types ADD COLUMN off_plot BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE station_types SET off_plot = TRUE WHERE id = 'workbench';

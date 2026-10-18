-- E0.3 (#146): a station type names its kit, the item that places it and that
-- taking it down gives back. It was a match in code, copied in four places.
-- The values come from backend/content/stations.toml, upserted at startup;
-- these rows only fill the column for the stations that exist today, so a
-- database is the same before and after its first content upsert.
ALTER TABLE station_types ADD COLUMN kit_item VARCHAR(50) REFERENCES item_types(id);

UPDATE station_types SET kit_item = 'workbench_kit' WHERE id = 'workbench';
UPDATE station_types SET kit_item = 'forge_kit' WHERE id = 'forge';
UPDATE station_types SET kit_item = 'chest_kit' WHERE id = 'storage_chest';

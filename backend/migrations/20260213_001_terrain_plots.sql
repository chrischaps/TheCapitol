-- B7 (#42): a claimable plot is a /terrain lot.
--
-- The terrain generator makes the lots. The server upserts them into this
-- table on startup and on every committed terrain change (engine/plot_sync.rs),
-- keyed by the lot's id. The UUID stays the primary key, so stations, tax and
-- permissions keep pointing at the same rows.

ALTER TABLE plots ADD COLUMN terrain_id TEXT UNIQUE;
-- The lot's corners, flattened: x0, y0, x1, y1, x2, y2, x3, y3. The bounds_*
-- columns hold its bounding box, which the index prefilters on.
ALTER TABLE plots ADD COLUMN corners DOUBLE PRECISION[];

-- Lots aren't on a grid. grid_x now holds the lot's index in /terrain order
-- and grid_y is 0, so the old grid key no longer applies.
ALTER TABLE plots DROP CONSTRAINT IF EXISTS plots_zone_id_grid_x_grid_y_key;

-- Capacity goes by the lot's area (engine/plot_sync.rs). Rural fields are far
-- bigger than any old plot, so they get a category of their own. The width
-- and height are nominal now.
INSERT INTO plot_size_categories (id, name, width, height, station_capacity)
VALUES ('estate', 'Estate', 170, 170, 25)
ON CONFLICT (id) DO NOTHING;

-- A plot removed by a forced terrain commit takes its tax history with it.
-- Its stations are removed first by the sync, so this only guards stray rows.
ALTER TABLE property_tax_transactions DROP CONSTRAINT IF EXISTS property_tax_transactions_plot_id_fkey;
ALTER TABLE property_tax_transactions ADD CONSTRAINT property_tax_transactions_plot_id_fkey
    FOREIGN KEY (plot_id) REFERENCES plots(id) ON DELETE CASCADE;
ALTER TABLE stations DROP CONSTRAINT IF EXISTS stations_plot_id_fkey;
ALTER TABLE stations ADD CONSTRAINT stations_plot_id_fkey
    FOREIGN KEY (plot_id) REFERENCES plots(id) ON DELETE SET NULL;

-- Replace the old grid plots wholesale. None are owned on any DB so far (the
-- fresh DB's 847 and the dev DB's 6417 alike). An owned one is kept as it is,
-- box and all, until its owner gives it up. The startup sync adds the lots.
UPDATE stations SET plot_id = NULL
WHERE plot_id IN (SELECT id FROM plots WHERE owner_id IS NULL);
DELETE FROM property_tax_transactions
WHERE plot_id IN (SELECT id FROM plots WHERE owner_id IS NULL);
DELETE FROM plots WHERE owner_id IS NULL;

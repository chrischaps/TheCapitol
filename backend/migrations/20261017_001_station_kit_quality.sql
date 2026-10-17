-- #88: taking a station down gives its kit back, so the station keeps the
-- quality of the kit that placed it. Stations placed before this get 50, the
-- middle of the range: their kit's quality was never recorded.
ALTER TABLE stations ADD COLUMN kit_quality INTEGER NOT NULL DEFAULT 50
    CHECK (kit_quality BETWEEN 0 AND 100);

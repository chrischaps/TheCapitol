-- The bazaar (E3.10, economy plan D6): stalls merchants rent across the
-- avenue from the Exchange hall, and the goods they list in them. The stalls
-- themselves are `/terrain` landmarks (`bazaar_stall_1` to `_13`), so a stall
-- is named here by that id.

-- A listing's units wait in a stack of their own in the bazaar's stock, with
-- no slot and no owner, as a sell order's do in the market's escrow: nobody
-- can move them but the bazaar. The listing names its stack.
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('bazaar_stock', 'Bazaar Stock', 0, 1);
CREATE UNIQUE INDEX idx_containers_one_bazaar_stock ON containers (container_type)
    WHERE container_type = 'bazaar_stock';
INSERT INTO containers (id, container_type, owner_id) VALUES (gen_random_uuid(), 'bazaar_stock', NULL);

-- A rented stall: one a player, and one merchant a stall. When the rent runs
-- out the row goes, its listings' goods go to the merchant's market box, and
-- the stall is free.
CREATE TABLE bazaar_rentals (
    stall_id TEXT PRIMARY KEY,
    player_id UUID NOT NULL UNIQUE REFERENCES players(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    rented_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    paid_until TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_bazaar_rentals_paid_until ON bazaar_rentals (paid_until);

-- A stack at a fixed price per unit. `stall_id` is NULL once the stall has
-- lapsed and the goods are waiting for room in the merchant's market box.
CREATE TABLE bazaar_listings (
    id UUID PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    stall_id TEXT,
    item_id UUID NOT NULL UNIQUE REFERENCES items(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    -- Strands per unit
    price BIGINT NOT NULL CHECK (price >= 1),
    listed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_bazaar_listings_stall ON bazaar_listings (stall_id) WHERE stall_id IS NOT NULL;
CREATE INDEX idx_bazaar_listings_returning ON bazaar_listings (player_id) WHERE stall_id IS NULL;

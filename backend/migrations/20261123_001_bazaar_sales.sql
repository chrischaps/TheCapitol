-- The bazaar's sales (#240): one row a purchase at a stall, written in the
-- buy's transaction, so the Gazette can count the stalls' trade as it counts
-- the Exchange's from `market_trades`. The edition prints counts and the
-- busiest shops, never a price (D6). The shop's name is kept as it was at
-- the sale, since a rental and its name go when the rent runs out.
CREATE TABLE bazaar_sales (
    id UUID PRIMARY KEY,
    -- The listing it came from; no foreign key, as a listing sold out goes
    listing_id UUID NOT NULL,
    stall_id TEXT NOT NULL,
    shop_name TEXT NOT NULL,
    merchant_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    buyer_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    -- The mean quality of the units bought
    quality INTEGER NOT NULL CHECK (quality BETWEEN 0 AND 100),
    -- Strands per unit
    price BIGINT NOT NULL CHECK (price >= 1),
    quantity INTEGER NOT NULL CHECK (quantity >= 1),
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_bazaar_sales_at ON bazaar_sales (at);

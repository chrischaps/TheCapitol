-- The market's model (E3.1, economy plan D7): orders, fills, the escrow
-- container and each player's market box. Matching is E3.2, the routes E3.3.

-- The escrow holds each sell order's units as a stack of their own, with no
-- slot and no owner, so nobody can move them out: the order names its stack.
-- The market box is where a cancelled or expired sell's units, and bought
-- goods, wait to be collected (D6).
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('market_escrow', 'Market Escrow', 0, 1),
    ('market_box', 'Market Box', 48, 8);

-- One escrow, shared by every order
CREATE UNIQUE INDEX idx_containers_one_market_escrow ON containers (container_type)
    WHERE container_type = 'market_escrow';
INSERT INTO containers (id, container_type, owner_id) VALUES (gen_random_uuid(), 'market_escrow', NULL);

-- One market box a player. New players get theirs the first time the market
-- needs it (engine::market::market_box).
CREATE UNIQUE INDEX idx_containers_one_market_box ON containers (owner_id)
    WHERE container_type = 'market_box';
INSERT INTO containers (id, container_type, owner_id)
    SELECT gen_random_uuid(), 'market_box', id FROM players;

CREATE TABLE market_orders (
    id UUID PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    side VARCHAR(4) NOT NULL CHECK (side IN ('buy', 'sell')),
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    -- A buy's grade and how it reads the units sold (D2, D7); NULL on a sell
    min_grade CHAR(1) CHECK (min_grade IN ('A', 'B', 'C', 'D', 'F')),
    grade_rule VARCHAR(10) CHECK (grade_rule IN ('average', 'every_unit')),
    -- Strands per unit
    price BIGINT NOT NULL CHECK (price >= 1),
    quantity_total INTEGER NOT NULL CHECK (quantity_total >= 1),
    quantity_remaining INTEGER NOT NULL CHECK (quantity_remaining BETWEEN 0 AND quantity_total),
    status VARCHAR(10) NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'filled', 'cancelled', 'expired')),
    -- A sell's units in escrow. A fill that takes the last of them deletes the
    -- stack, and this goes NULL.
    escrow_item_id UUID REFERENCES items(id) ON DELETE SET NULL,
    -- A buy's Strands in escrow. A fill at a lower resting price leaves some
    -- over, so it's kept, not worked out from the price.
    escrow_strands BIGINT NOT NULL DEFAULT 0 CHECK (escrow_strands >= 0),
    -- The listing fee paid at placement (1%), not refunded on cancel
    fee BIGINT NOT NULL CHECK (fee >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    CHECK ((side = 'buy') = (min_grade IS NOT NULL AND grade_rule IS NOT NULL)),
    CHECK (side = 'buy' OR escrow_strands = 0),
    CHECK (side = 'sell' OR escrow_item_id IS NULL)
);

-- The book: open orders of a type, by price then time
CREATE INDEX idx_market_orders_book ON market_orders (item_type, side, price, created_at)
    WHERE status = 'open';
CREATE INDEX idx_market_orders_player ON market_orders (player_id, created_at DESC);
CREATE INDEX idx_market_orders_expiry ON market_orders (expires_at) WHERE status = 'open';

CREATE TABLE market_trades (
    id UUID PRIMARY KEY,
    buy_order_id UUID NOT NULL REFERENCES market_orders(id) ON DELETE CASCADE,
    sell_order_id UUID NOT NULL REFERENCES market_orders(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    -- The grade of `quality`, the mean of the units traded; `min_quality` is
    -- the lowest of them, for an every-unit chart
    grade CHAR(1) NOT NULL,
    quality INTEGER NOT NULL CHECK (quality BETWEEN 0 AND 100),
    min_quality INTEGER NOT NULL CHECK (min_quality BETWEEN 0 AND quality),
    -- Strands per unit: the resting order's price
    price BIGINT NOT NULL CHECK (price >= 1),
    quantity INTEGER NOT NULL CHECK (quantity >= 1),
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- History and the chart: a type's fills over time
CREATE INDEX idx_market_trades_history ON market_trades (item_type, at);

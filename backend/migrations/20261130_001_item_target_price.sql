-- Target prices (D2.3, #362): Strands a unit the economy is balanced to,
-- D5's value chain, from `target_price` in content/items.toml. Market makers
-- quote round it before an item has traded, and the Observatory charts the
-- market's VWAP against it and watches for an item off target. NULL where
-- the content gives none.
ALTER TABLE item_types ADD COLUMN target_price BIGINT CHECK (target_price >= 1);

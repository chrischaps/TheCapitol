-- E1.4 (#156): hand tools. An item type is a tool when it names a tool_type;
-- its tier names the material, and base_power and max_durability are what it
-- does and how long it lasts at Q50 (protocol rules::tools scales both by
-- quality). The values come from backend/content/items.toml, upserted at
-- startup. A tool's item carries the durability it has left.
ALTER TABLE item_types
    ADD COLUMN tool_type VARCHAR(20),
    ADD COLUMN tier VARCHAR(20),
    ADD COLUMN base_power DOUBLE PRECISION,
    ADD COLUMN max_durability INTEGER,
    ADD CONSTRAINT item_types_tool_all_or_none CHECK (
        (tool_type IS NULL AND tier IS NULL AND base_power IS NULL AND max_durability IS NULL)
        OR (tool_type IS NOT NULL AND tier IS NOT NULL AND base_power > 0 AND max_durability > 0
            AND NOT stackable)
    );

ALTER TABLE items ADD COLUMN durability INTEGER CHECK (durability >= 0);

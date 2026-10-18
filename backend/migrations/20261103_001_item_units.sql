-- E1.8 (#192): every unit keeps its own quality (plan D2). A stack is a count
-- per quality, in item_units, and that's the truth. items.quantity,
-- items.quality (the mean, rounded down) and items.min_quality are summaries
-- of it, kept in step by one writer, item_units_summarise below. The frozen
-- React and Love2D clients read them unchanged.
CREATE TABLE item_units (
    item_id  UUID     NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    quality  SMALLINT NOT NULL CHECK (quality BETWEEN 0 AND 100),
    count    INTEGER  NOT NULL CHECK (count > 0),
    PRIMARY KEY (item_id, quality)
);

-- Each stack becomes one row. Stacks blended before this lost their history
-- with E1.2's averaging, so they become uniform at their mean.
INSERT INTO item_units (item_id, quality, count)
SELECT id, LEAST(GREATEST(quality, 0), 100), quantity FROM items WHERE quantity > 0;
UPDATE items SET quality = LEAST(GREATEST(quality, 0), 100), min_quality = LEAST(GREATEST(quality, 0), 100);

-- min_quality is derived now: E1.2's trigger goes
DROP TRIGGER items_min_quality ON items;
DROP FUNCTION items_min_quality();

-- The one writer: whenever a stack's units change, its summaries follow. A
-- stack whose last unit goes keeps its old summaries; whoever took the last
-- unit deletes the item (engine::units does).
CREATE FUNCTION item_units_summarise() RETURNS trigger AS $$
DECLARE
    target UUID := CASE WHEN TG_OP = 'DELETE' THEN OLD.item_id ELSE NEW.item_id END;
BEGIN
    UPDATE items i
    SET quantity = s.n, quality = div(s.total, s.n)::INTEGER, min_quality = s.lo
    -- SUM gives NUMERIC: div() rounds the mean down, as rules::stacking does
    FROM (SELECT SUM(count) AS n, SUM(quality::BIGINT * count) AS total, MIN(quality)::INTEGER AS lo
          FROM item_units WHERE item_id = target) s
    WHERE i.id = target AND s.n > 0;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER item_units_summarise
    AFTER INSERT OR UPDATE OR DELETE ON item_units
    FOR EACH ROW EXECUTE FUNCTION item_units_summarise();

-- A new item is uniform: its units are `quantity` at `quality`, and its min is
-- its quality. An insert that copies a mixed stack (a split, a trade) replaces
-- these units straight after (engine::units::insert_stack).
CREATE FUNCTION items_fresh_min() RETURNS trigger AS $$
BEGIN
    NEW.min_quality := NEW.quality;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER items_fresh_min
    BEFORE INSERT ON items
    FOR EACH ROW EXECUTE FUNCTION items_fresh_min();

CREATE FUNCTION items_seed_units() RETURNS trigger AS $$
BEGIN
    IF NEW.quantity > 0 THEN
        INSERT INTO item_units (item_id, quality, count) VALUES (NEW.id, NEW.quality, NEW.quantity);
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER items_seed_units
    AFTER INSERT ON items
    FOR EACH ROW EXECUTE FUNCTION items_seed_units();

-- Nothing else writes the summaries. A statement that sets them straight
-- would part them from the units, so it's refused: write item_units instead.
-- pg_trigger_depth() is 1 for a statement's own update and more for
-- item_units_summarise's.
CREATE FUNCTION items_summaries_follow_units() RETURNS trigger AS $$
BEGIN
    IF pg_trigger_depth() = 1 AND (
        NEW.quantity IS DISTINCT FROM OLD.quantity
        OR NEW.quality IS DISTINCT FROM OLD.quality
        OR NEW.min_quality IS DISTINCT FROM OLD.min_quality
    ) THEN
        RAISE EXCEPTION 'items.quantity, quality and min_quality follow item_units: write item_units (E1.8)';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER items_summaries_follow_units
    BEFORE UPDATE OF quantity, quality, min_quality ON items
    FOR EACH ROW EXECUTE FUNCTION items_summaries_follow_units();

-- E1.2 (#154): each stack carries the lowest quality blended into it (plan D2).
-- A merge takes the lower of the two mins; a split keeps both values; a fresh
-- item starts with min = quality. Existing stacks are each their own origin.
ALTER TABLE items ADD COLUMN min_quality INTEGER;
UPDATE items SET min_quality = quality;
ALTER TABLE items ALTER COLUMN min_quality SET NOT NULL;

-- Most inserts make a fresh item and don't name min_quality, so it defaults to
-- the quality here. An insert that copies a stack (a split, a trade) names it.
-- The mean of a stack is never below its min, so a write that leaves the min
-- above the quality lowers it to match.
CREATE FUNCTION items_min_quality() RETURNS trigger AS $$
BEGIN
    NEW.min_quality := LEAST(COALESCE(NEW.min_quality, NEW.quality), NEW.quality);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER items_min_quality
    BEFORE INSERT OR UPDATE OF quality, min_quality ON items
    FOR EACH ROW EXECUTE FUNCTION items_min_quality();

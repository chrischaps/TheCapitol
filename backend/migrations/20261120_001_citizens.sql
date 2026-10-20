-- Citizens (C0.1, #246): an account is a person's or a citizen's, an AI
-- player run by capitol-town. Only `POST /admin/citizens` makes a citizen;
-- registration makes a human, and an account never changes kind, so the
-- mark other players see (`citizen` on the wire) can't be put on or taken off.
ALTER TABLE accounts
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'human'
    CHECK (kind IN ('human', 'citizen'));

CREATE FUNCTION accounts_kind_is_fixed() RETURNS trigger AS $$
BEGIN
    IF NEW.kind IS DISTINCT FROM OLD.kind THEN
        RAISE EXCEPTION 'An account''s kind never changes (% to %)', OLD.kind, NEW.kind;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER accounts_kind_is_fixed
    BEFORE UPDATE OF kind ON accounts
    FOR EACH ROW EXECUTE FUNCTION accounts_kind_is_fixed();

-- The admin's roster (`GET /admin/citizens`)
CREATE INDEX idx_accounts_citizens ON accounts (created_at) WHERE kind = 'citizen';

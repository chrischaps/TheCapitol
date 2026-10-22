-- Letters from the city's offices (#447, road-in plan §4.4): the Registrar's
-- letter, which opens the road in's People chapter. An office isn't a
-- player (plan D1: the bureaucracy's voice, never a character), so such a
-- letter has no sender_id, and names its office instead. It keeps no out
-- box: the office's copy is never kept.
ALTER TABLE letters ALTER COLUMN sender_id DROP NOT NULL;
ALTER TABLE letters ADD COLUMN from_office TEXT;
ALTER TABLE letters ADD CONSTRAINT letters_one_writer CHECK ((sender_id IS NULL) <> (from_office IS NULL));
ALTER TABLE letters ADD CONSTRAINT letters_office_keeps_no_copy CHECK (from_office IS NULL OR NOT kept_by_sender);

-- Each office letter goes once an account, whatever becomes of the letter
-- itself: dropped, trimmed from a full box or never read, it isn't sent
-- again. `letter` names which (`registrar_welcome`).
CREATE TABLE office_letters (
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    letter TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_id, letter)
);

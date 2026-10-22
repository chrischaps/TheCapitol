-- Letters (commons plan D4, S1.5): a letter waits for someone who's away.
-- Each side keeps its own copy: the writer's out box and the reader's in
-- box drop theirs apart, and the row goes once neither keeps it. Each box
-- keeps `rules::talk::MAILBOX` letters; past it the oldest read go first.
CREATE TABLE letters (
    id UUID PRIMARY KEY,
    sender_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    recipient_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    -- Cleaned by `rules::talk::clean_letter`: up to 1000 characters, its
    -- paragraphs kept
    body TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- When the reader opened it
    read_at TIMESTAMPTZ,
    kept_by_sender BOOLEAN NOT NULL DEFAULT TRUE,
    kept_by_recipient BOOLEAN NOT NULL DEFAULT TRUE,
    CHECK (sender_id <> recipient_id),
    CHECK (kept_by_sender OR kept_by_recipient)
);

-- Each box, newest first
CREATE INDEX idx_letters_in ON letters (recipient_id, sent_at DESC, id DESC) WHERE kept_by_recipient;
CREATE INDEX idx_letters_out ON letters (sender_id, sent_at DESC, id DESC) WHERE kept_by_sender;

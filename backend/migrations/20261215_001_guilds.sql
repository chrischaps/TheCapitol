-- Guilds (commons plan D10, S4.1): the registry, the roster and who's
-- waiting to join. A subscriber founds one at the Exchange hall for 500
-- Strands (`rules::guild::FOUNDING_FEE`); each roster change costs 1.
CREATE TABLE guilds (
    id UUID PRIMARY KEY,
    -- Cleaned by `rules::guild::clean_name`: 3 to 40 characters, no
    -- reserved word. Unique ignoring case among guilds that stand
    name TEXT NOT NULL,
    -- 2 to 5 letters, in capitals (`clean_tag`)
    tag TEXT NOT NULL CHECK (tag ~ '^[A-Z]{2,5}$'),
    charter TEXT NOT NULL DEFAULT '',
    founded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    founder_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    leader_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    -- When the lead came to them: a leader who never subscribed counts as
    -- lapsed from here, for the lead to pass on after four weeks
    leader_since TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 'disbanded' once its last member leaves; the row stays for the record
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disbanded')),
    disbanded_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_guilds_name ON guilds (lower(name)) WHERE status = 'active';
CREATE UNIQUE INDEX idx_guilds_tag ON guilds (tag) WHERE status = 'active';

-- One guild per player: the player is the key
CREATE TABLE guild_members (
    player_id UUID PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
    guild_id UUID NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('leader', 'officer', 'member')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_guild_members_guild ON guild_members (guild_id, joined_at);
-- One leader a guild
CREATE UNIQUE INDEX idx_guild_members_leader ON guild_members (guild_id) WHERE role = 'leader';

-- An invitation (an officer or the leader asked) or an application (the
-- player asked). Either waits for the other side, whose same action
-- accepts it.
CREATE TABLE guild_invites (
    guild_id UUID NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    by_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('invite', 'apply')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, player_id)
);

CREATE INDEX idx_guild_invites_player ON guild_invites (player_id);

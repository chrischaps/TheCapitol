-- Annotations (the Observatory's D0.4, #354): the things that explain a bend
-- in a chart, which the admin console draws as markers. `kind` is
-- `capitol_protocol::annotation::AnnotationKind`'s snake_case name; `text` a
-- plain sentence for the marker. Written by `engine::annotations::record`.
CREATE TABLE admin_events (
    id BIGSERIAL PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    kind TEXT NOT NULL CHECK (kind IN (
        'boot', 'terrain_committed', 'gazette_published', 'tunable_changed',
        'town_up', 'town_down', 'watch_fired'
    )),
    text TEXT NOT NULL
);
CREATE INDEX idx_admin_events_at ON admin_events (at);

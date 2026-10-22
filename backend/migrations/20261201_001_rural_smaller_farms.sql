-- #423: about 400 farms of about 36k instead of 117 of about 117k (plots
-- plan §5.3, rural plan §5). The farms are long lots now: narrower and
-- closer along the same lanes. So:
--
-- 1. A saved `world_config` that still carries R5's draws takes the new ones,
--    so it builds the default world. A world an admin tuned keeps its own.
--
-- 2. Every plot on a farm of the old layout is renamed `oldfarm_…`, so the
--    plot sync can tell it from the new farms, which keep the `farm_…` ids.
--    No new farm has an old one's shape, so nothing is lost by the rename:
--    startup's sync deletes the unowned rows and lays the new farms, and an
--    owned one is released as a retired layout's (`plot_sync::RETIRED_LOTS`),
--    with the deed refunded and its stations and their contents given back,
--    as R7's flip released the trails' lots (#290). A delete here would
--    destroy the stations' contents unbooked.
--
-- The planned nodes the farms move are re-planned after the content sync, in
-- post-content/20261201_001_rural_smaller_farms_nodes.sql. No seeded grass
-- moves (`the_default_terrain_moves_no_seeded_node`).

UPDATE world_config
SET params = jsonb_set(jsonb_set(jsonb_set(params,
    '{rural,frontage}', '[120.0, 160.0]'),
    '{rural,field_gap}', '[50.0, 100.0]'),
    '{rural,depth}', '[230.0, 310.0]')
WHERE params->'rural'->'frontage' = '[380.0, 500.0]'::jsonb
  AND params->'rural'->'field_gap' = '[240.0, 440.0]'::jsonb
  AND params->'rural'->'depth' = '[280.0, 380.0]'::jsonb;

UPDATE plots SET terrain_id = 'old' || terrain_id WHERE terrain_id LIKE 'farm\_%';

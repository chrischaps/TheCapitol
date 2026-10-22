-- #422: about 1200 suburban lots of about 10k instead of 1905 of about 6.7k
-- (plots plan §5.2, suburb plan §4.4). The streets are the same; the lots
-- along them are about 100 wide, not 64, and a bulb has 2 or 3 pies, not 4
-- or 5, so most lots take 4 stations. So:
--
-- 1. A saved `world_config` that still carries the old frontage takes the
--    new one, so it builds the default world. A world an admin tuned keeps
--    its own.
--
-- 2. Every plot on a suburban lot of the old layout is renamed `oldsub_…`,
--    so the plot sync can tell it from the new lots. The new lots keep the
--    `sub_{sector}_{nbhd}_{street}_{L|R|B}_{k}` scheme, so most of the old
--    ids name a different lot now. Startup's sync deletes the unowned rows and
--    lays the new lots, and an owned one is released as a retired layout's
--    (`plot_sync::RETIRED_LOTS`), with the deed refunded and its stations and
--    their contents given back, as #423 did for the farms. A delete here
--    would destroy the stations' contents unbooked.
--
-- The planned nodes the lots move are re-planned after the content sync, in
-- post-content/20261205_001_suburb_wider_lots_nodes.sql. No seeded grass
-- moves (`the_default_terrain_moves_no_seeded_node`).

UPDATE world_config
SET params = jsonb_set(params, '{suburban,neighbourhood,lot_frontage}', '100.0')
WHERE params->'suburban'->'neighbourhood'->'lot_frontage' = '64.0'::jsonb;

UPDATE plots SET terrain_id = 'old' || terrain_id WHERE terrain_id LIKE 'sub\_%';

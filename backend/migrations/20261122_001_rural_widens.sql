-- RW1 (#308): the rural ring widens to r4600, and the world grows from 12000
-- to 13600 so the wilds keep their 2200 depth (rural plan RD10).
--
-- The centre moves from (6000, 6000) to (6800, 6800)
-- (capitol_protocol::rules::CENTER_X/Y), so every stored position moves by
-- +800 on both axes and keeps its place relative to the tower, as in W1's
-- 20261107_001_world_grows.sql. The zone radii are measured from the centre,
-- so nothing inside the suburb's edge changes. Past r3800 the land, the wild
-- water, the trails and the nodes are new: the nodes are re-planned by
-- post-content/20261122_001_rural_widens_nodes.sql, after the content sync.
--
-- Every table with a position:
--   players               position, destination
--   resource_nodes        position
--   resource_node_seeds   position, while it exists: a fresh database has it
--                         until post-content's 20261116 plants and drops it
--   plots                 centre, bounds and corners (grid_x/y are indices)
--   stations              position
-- The tables added since W1 (the market, the bazaar, the Gazette, impound,
-- citizens) hold no position. `stations.automation` holds none either.
--
-- A saved `world_config` holds no position, but its land's radii go out
-- with the rural edge too: the foothills from r3900 to r4700, the ranges
-- full height from r5600 to r6400 (`land::LandParams`). The `zones` rows are
-- synced from the rules on startup (#116).

UPDATE players SET
    position_x = position_x + 800,
    position_y = position_y + 800,
    destination_x = destination_x + 800,
    destination_y = destination_y + 800;

UPDATE resource_nodes SET
    position_x = position_x + 800,
    position_y = position_y + 800;

DO $$
BEGIN
    IF to_regclass('resource_node_seeds') IS NOT NULL THEN
        UPDATE resource_node_seeds SET
            position_x = position_x + 800,
            position_y = position_y + 800;
    END IF;
END $$;

-- `corners` is x0, y0, x1, y1, ...: every element moves by the same amount.
UPDATE plots SET
    world_x = world_x + 800,
    world_y = world_y + 800,
    bounds_min_x = bounds_min_x + 800,
    bounds_min_y = bounds_min_y + 800,
    bounds_max_x = bounds_max_x + 800,
    bounds_max_y = bounds_max_y + 800,
    corners = (SELECT array_agg(c + 800 ORDER BY i) FROM unnest(corners) WITH ORDINALITY AS t(c, i));

UPDATE stations SET
    position_x = position_x + 800,
    position_y = position_y + 800;

UPDATE world_config SET params = jsonb_set(
    jsonb_set(params, '{land,foothill_radius}', to_jsonb((params->'land'->>'foothill_radius')::float8 + 800)),
    '{land,range_radius}', to_jsonb((params->'land'->>'range_radius')::float8 + 800))
WHERE params->'land' ? 'foothill_radius' AND params->'land' ? 'range_radius';

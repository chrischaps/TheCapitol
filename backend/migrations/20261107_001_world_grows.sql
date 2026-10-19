-- W1 (#194): the world grows from 8000 to 12000 for the wilds.
--
-- The centre moves from (4000, 4000) to (6000, 6000)
-- (capitol_protocol::rules::CENTER_X/Y), so every stored position moves by
-- +2000 on both axes and keeps its place relative to the tower. Nothing inside
-- the rural edge (r3800) changes size: the zone radii are measured from the
-- centre, and the terrain generator works round it.
--
-- Every table with a position:
--   players               position, destination
--   resource_nodes        position
--   resource_node_seeds   position (E2.3's rows, planted on startup, #207)
--   plots                 centre, bounds and corners
--   stations              position
-- `world_config.params` holds radii and widths but no position, so a saved
-- world needs nothing. The `zones` rows are synced from the rules on startup
-- (#116), which takes the wilderness out to r6000.
--
-- The seeded-node tests read the earlier migrations' rows and add this shift
-- (`WORLD_SHIFT` in models/resource.rs), so they check where the rows stand
-- now.

UPDATE players SET
    position_x = position_x + 2000,
    position_y = position_y + 2000,
    destination_x = destination_x + 2000,
    destination_y = destination_y + 2000;

UPDATE resource_nodes SET
    position_x = position_x + 2000,
    position_y = position_y + 2000;

UPDATE resource_node_seeds SET
    position_x = position_x + 2000,
    position_y = position_y + 2000;

-- `corners` is x0, y0, x1, y1, ...: every element moves by the same amount.
UPDATE plots SET
    world_x = world_x + 2000,
    world_y = world_y + 2000,
    bounds_min_x = bounds_min_x + 2000,
    bounds_min_y = bounds_min_y + 2000,
    bounds_max_x = bounds_max_x + 2000,
    bounds_max_y = bounds_max_y + 2000,
    corners = (SELECT array_agg(c + 2000 ORDER BY i) FROM unnest(corners) WITH ORDINALITY AS t(c, i));

UPDATE stations SET
    position_x = position_x + 2000,
    position_y = position_y + 2000;

-- R7 (#290): the rural ring is laid as farms on lanes (rural plan §7). The
-- default `RuralParams::layout` is now `Farms`, so:
--
-- 1. A saved `world_config` that names the old trails builds the farms too.
--    `trails` was only ever the default written back by an admin's PUT, never
--    a choice, and R8 (#291) removes the layout.
--
-- 2. Owned plots on the old trails' lots (`rural_plot_%`) aren't released
--    here. Their stations hold kits, tools and stock, which a delete would
--    destroy unbooked. Startup's plot sync releases them instead
--    (`plot_sync::RETIRED_LOTS`): the deed refunded, the stations and their
--    contents given back to inventories or impound, and the farms laid.
--
-- 3. Two grass nodes the new lanes run over move to the nearest spot in their
--    zone that is clear of every road, lot and node, as
--    `engine::node_placement::place_nodes` would. The rows come from the
--    ignored test `print_node_moves`, on the 13600 world. E2.3's planned
--    nodes are re-planned after the content sync, in
--    post-content/20261126_001_rural_farms_nodes.sql.

UPDATE world_config SET params = jsonb_set(params, '{rural,layout}', '"farms"')
WHERE params->'rural'->>'layout' = 'trails';

-- Matched on the previous position, so a node moved by hand stays put, and
-- running this twice does nothing.
UPDATE resource_nodes AS n
SET position_x = v.nx, position_y = v.ny
FROM (VALUES
    (6948.4, 10159.6, 6955.8, 10188.7),
    (3608.3, 7806.6, 3574.7, 7816.5)
) AS v(x, y, nx, ny)
WHERE n.resource_type = 'grass' AND n.position_x = v.x AND n.position_y = v.y;

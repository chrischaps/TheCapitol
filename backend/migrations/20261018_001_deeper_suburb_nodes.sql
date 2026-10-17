-- #117: the suburb deepened from 1600-2400 to 1800-3000, and the urban zone
-- grew out to 1800. That moves 31 of the grass nodes seeded by
-- 20260212_001_world_scale.sql (from where 20260215_001 left them):
--
-- - The 16 that now fall in the urban zone (radius 1656-1773) step out along
--   their own radius, back into the suburb: 200 out, or further in steps of 5
--   until the spot is 30 clear of every road, off every lot and 30 from other
--   nodes. Most stop at 200, the furthest at 490. Two found no spot off a lot
--   and stand on one, as #102 lets a node do.
-- - Then any node the new default world's roads run over moves to the nearest
--   spot in its zone that is 30 clear of every road, off every lot, and 30
--   from other nodes, as `engine::node_placement::place_nodes` would.
--
-- Startup's `node_placement` would make the second kind of move anyway, but
-- not the first. `models::resource::tests` check the seeds against the
-- default world, applying the move migrations in order.
--
-- Named 20261018 though written on 2026-10-17: sqlx takes the version from
-- the digits before the first underscore, and 20261017 is taken.
--
-- Matched on the previous position, so a node moved by hand stays put, and
-- running this twice does nothing.
UPDATE resource_nodes AS n
SET position_x = v.nx, position_y = v.ny
FROM (VALUES
    (3365.5, 2425.8, 3272.0, 2193.9),
    (2428.2, 4787.1, 2249.4, 4876.7),
    (2479.5, 5402.4, 2417.5, 5493.3),
    (5349.1, 5067.0, 5560.9, 5234.5),
    (2447.3, 3180.0, 2164.3, 3030.6),
    (6236.5, 4550.7, 6271.4, 4586.5),
    (5613.6, 4700.7, 5797.0, 4780.4),
    (3579.1, 2371.3, 3529.1, 2177.7),
    (5612.1, 3513.3, 5918.4, 3420.8),
    (5578.3, 4504.2, 5768.8, 4565.1),
    (5541.9, 2668.6, 5595.1, 2631.2),
    (3104.2, 6036.9, 3118.1, 6057.7),
    (4405.9, 2315.7, 4452.8, 2121.3),
    (4489.6, 1772.7, 4482.2, 1718.2),
    (5913.9, 4540.2, 5943.9, 4540.2),
    (3652.7, 2034.6, 3640.6, 2007.1),
    (2760.5, 2855.0, 2613.6, 2719.3),
    (2366.0, 4566.4, 2134.5, 4646.6),
    (5773.3, 3940.9, 5973.2, 3934.2),
    (5518.2, 5042.9, 5495.3, 5016.4),
    (2326.7, 3557.3, 2080.2, 3492.1),
    (2332.6, 3841.1, 2133.5, 3822.1),
    (2233.3, 4158.9, 2034.1, 4176.8),
    (5977.6, 3499.7, 6015.9, 3523.3),
    (4667.8, 2453.9, 4862.1, 2004.1),
    (1954.0, 5590.2, 1978.5, 5595.1),
    (6316.9, 5004.9, 6352.4, 4977.3),
    (1596.7, 5577.5, 1631.0, 5620.5),
    (1390.5, 4257.7, 1343.3, 4285.9),
    (5860.3, 5607.1, 5940.1, 5612.1),
    (6800.4, 3832.0, 6789.5, 3793.5)
) AS v(x, y, nx, ny)
WHERE n.resource_type = 'grass' AND n.position_x = v.x AND n.position_y = v.y;

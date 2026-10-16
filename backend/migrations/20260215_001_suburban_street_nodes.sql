-- #62: the suburban street generator was rebuilt, and some new streets run
-- over grass nodes seeded by 20260212_001_world_scale.sql. Move those 19 nodes
-- to the nearest spot in the same zone that is 30 units clear of every road in
-- the default world, off every lot, and 30 units from other nodes.
--
-- Nodes are still placed against the default terrain, not the live one (#70).
-- `models::resource::tests::seeded_nodes_sit_clear_of_the_default_terrain`
-- applies these moves on top of the seeds.
--
-- Matched on the seeded position, so a node that was moved by hand stays put,
-- and running this twice does nothing.
UPDATE resource_nodes AS n
SET position_x = v.nx, position_y = v.ny
FROM (VALUES
    (4285.9, 1812.8, 4298.4, 1791.1),
    (2843.4, 5706.3, 2793.8, 5712.8),
    (5937.9, 2867.7, 5923.8, 2881.8),
    (2462.6, 3217.0, 2447.3, 3180.0),
    (2950.4, 2463.3, 2936.3, 2449.2),
    (5784.0, 2753.8, 5822.9, 2792.7),
    (1870.1, 3479.6, 1830.5, 3531.2),
    (5606.2, 3557.9, 5612.1, 3513.3),
    (3106.8, 6017.1, 3104.2, 6036.9),
    (2094.4, 2822.3, 2091.1, 2847.1),
    (3668.0, 2071.6, 3652.7, 2034.6),
    (1959.3, 3330.3, 1925.5, 3321.2),
    (2433.2, 2576.5, 2467.9, 2581.1),
    (4334.3, 5821.0, 4329.1, 5801.7),
    (2088.9, 4823.8, 2079.8, 4811.9),
    (5406.3, 5787.3, 5406.3, 5802.3),
    (6233.9, 3584.3, 6298.6, 3611.1),
    (5997.0, 4912.4, 6036.7, 4917.6),
    (6020.9, 3474.7, 5977.6, 3499.7)
) AS v(x, y, nx, ny)
WHERE n.resource_type = 'grass' AND n.position_x = v.x AND n.position_y = v.y;

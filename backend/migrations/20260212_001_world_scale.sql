-- B1: move the world to the 0-8000 scale, centred on (4000, 4000).
--
-- The terrain generator, the zone rules (capitol_protocol::rules::Zone::radii)
-- and every client already use 8000. The data didn't: new players arrived at
-- (500, 500), off the edge of the world disc, the grass nodes sat in the same
-- corner, the zones table still had the 2000-world radii and the DB plots were
-- centred on (1000, 1000). Some databases were fixed by hand, so every step
-- here is safe to run on either state.

-- Zones: the same radii as Zone::radii().
UPDATE zones SET inner_radius = 0,    outer_radius = 200  WHERE id = 'capitol';
UPDATE zones SET inner_radius = 200,  outer_radius = 600  WHERE id = 'trade_district';
UPDATE zones SET inner_radius = 600,  outer_radius = 1000 WHERE id = 'guild_district';
UPDATE zones SET inner_radius = 1000, outer_radius = 1600 WHERE id = 'urban';
UPDATE zones SET inner_radius = 1600, outer_radius = 2400 WHERE id = 'suburban';
UPDATE zones SET inner_radius = 2400, outer_radius = 3400 WHERE id = 'rural';
UPDATE zones SET inner_radius = 3400, outer_radius = 4000 WHERE id = 'wilderness';

-- Plots and stations still on the 2000 world: scale their centres 4x about
-- the centre, (1000, 1000) -> (4000, 4000), which is just (x, y) -> (4x, 4y).
-- The old zone radii times 4 are the new ones exactly, so every plot lands in
-- its own zone's ring. Plot sizes stay as they are, so the grid is sparser
-- than before. That's deliberate: B7 (#42)
-- replaces these plots with the terrain plots, and regenerating them here as
-- well would be thrown away. Ids, owners and placed stations all survive.
DO $$
BEGIN
    IF (SELECT MAX(GREATEST(bounds_max_x, bounds_max_y)) FROM plots) <= 2100 THEN
        -- Stations first, while their plots still have the old centres. A
        -- station on a plot moves with it and keeps its offset from the plot's
        -- centre (4000 + 4 * (c - 1000) is 4c, so the shift is 3c). One with no
        -- plot scales like the plots do.
        UPDATE stations s SET
            position_x = s.position_x + 3 * p.world_x,
            position_y = s.position_y + 3 * p.world_y
        FROM plots p
        WHERE s.plot_id = p.id;
        UPDATE stations SET
            position_x = 4 * position_x,
            position_y = 4 * position_y
        WHERE plot_id IS NULL;

        UPDATE plots SET
            world_x = 4 * world_x,
            world_y = 4 * world_y,
            bounds_min_x = 4 * world_x - (bounds_max_x - bounds_min_x) / 2,
            bounds_max_x = 4 * world_x + (bounds_max_x - bounds_min_x) / 2,
            bounds_min_y = 4 * world_y - (bounds_max_y - bounds_min_y) / 2,
            bounds_max_y = 4 * world_y + (bounds_max_y - bounds_min_y) / 2;
    END IF;
END $$;

-- Players from the old world (its 0-2000 square), or off the world disc, move
-- to the spawn (capitol_protocol::rules::SPAWN_X/SPAWN_Y) and stop whatever
-- they were doing.
UPDATE players SET
    position_x = 4000,
    position_y = 7350,
    destination_x = NULL,
    destination_y = NULL,
    action_state = 'idle',
    action_target_id = NULL,
    action_progress = 0,
    current_zone = 'rural'
WHERE (position_x < 2000 AND position_y < 2000)
   OR sqrt(power(position_x - 4000, 2) + power(position_y - 4000, 2)) > 4000;

-- Resource nodes: replace the 50 corner nodes with 156 spread through the
-- suburban and rural zones. The positions came from the terrain generator's
-- default world (seed 42), chosen to stand at least 20 units clear of every
-- road. The test `seeded_nodes_sit_clear_of_the_default_terrain` in
-- models/resource.rs checks them against the generator, so a terrain change
-- that runs a road over one fails there.
--   starter: 6 nodes 60-170 from the spawn, so the first extraction is a short
--            walk off the road. Quality 40-55.
--   suburban: 60 nodes, r 1650-2350, at least 160 apart. Quality 40-60.
--   rural: 90 nodes, r 2450-3350, at least 200 apart. Quality 50-70, so the
--          better fiber is further out.
UPDATE players SET action_state = 'idle', action_target_id = NULL, action_progress = 0
WHERE action_target_id IN (SELECT id FROM resource_nodes);
DELETE FROM resource_nodes;

INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality, state)
SELECT gen_random_uuid(), 'grass', v.x, v.y, v.q, 'available'
FROM (VALUES
    -- starter
    (3880.3, 7381.5, 47),
    (4106.4, 7470.0, 46),
    (3881.3, 7309.4, 46),
    (3948.4, 7484.6, 54),
    (4148.4, 7359.6, 44),
    (4077.3, 7297.7, 42),
    -- suburban
    (4285.9, 1812.8, 51),
    (2843.4, 5706.3, 41),
    (3365.5, 2425.8, 46),
    (1811.4, 4324.5, 41),
    (4073.9, 6160.9, 49),
    (2558.6, 2398.8, 56),
    (2428.2, 4787.1, 54),
    (5937.9, 2867.7, 58),
    (2140.6, 5387.6, 55),
    (5198.2, 2187.2, 41),
    (2479.5, 5402.4, 60),
    (5349.1, 5067.0, 54),
    (2462.6, 3217.0, 55),
    (2950.4, 2463.3, 53),
    (5784.0, 2753.8, 49),
    (6236.5, 4550.7, 54),
    (5613.6, 4700.7, 52),
    (2214.5, 4417.1, 41),
    (1870.1, 3479.6, 58),
    (3579.1, 2371.3, 42),
    (5606.2, 3557.9, 49),
    (5578.3, 4504.2, 60),
    (5541.9, 2668.6, 50),
    (3106.8, 6017.1, 58),
    (2901.7, 5542.9, 45),
    (5234.8, 2523.7, 60),
    (4405.9, 2315.7, 58),
    (4489.6, 1772.7, 45),
    (4942.5, 2387.9, 58),
    (5059.1, 5636.7, 48),
    (2094.4, 2822.3, 53),
    (5913.9, 4540.2, 59),
    (3668.0, 2071.6, 54),
    (2658.0, 5223.1, 56),
    (2367.0, 5262.5, 43),
    (1959.3, 3330.3, 41),
    (2760.5, 2855.0, 52),
    (2181.5, 2512.7, 44),
    (3897.1, 1940.1, 55),
    (2433.2, 2576.5, 40),
    (4334.3, 5821.0, 44),
    (3504.4, 1942.1, 47),
    (2366.0, 4566.4, 44),
    (2088.9, 4823.8, 54),
    (5773.3, 3940.9, 55),
    (5518.2, 5042.9, 43),
    (6050.1, 3172.8, 45),
    (5522.9, 5420.2, 51),
    (6145.9, 4061.3, 47),
    (2326.7, 3557.3, 57),
    (2332.6, 3841.1, 59),
    (5406.3, 5787.3, 50),
    (6233.9, 3584.3, 48),
    (2233.3, 4158.9, 57),
    (5997.0, 4912.4, 49),
    (3015.7, 5761.3, 48),
    (1976.1, 4268.3, 56),
    (1899.2, 3140.2, 55),
    (6020.9, 3474.7, 57),
    (4667.8, 2453.9, 43),
    -- rural
    (2508.5, 1204.7, 60),
    (808.3, 5006.6, 62),
    (5804.6, 6755.0, 51),
    (3212.1, 1115.3, 64),
    (2744.2, 1254.2, 67),
    (1458.0, 3518.7, 54),
    (5928.7, 1859.5, 52),
    (6504.7, 3808.7, 54),
    (912.5, 2965.7, 67),
    (2274.4, 6368.9, 51),
    (1954.0, 5590.2, 54),
    (6934.3, 3154.0, 64),
    (6316.9, 5004.9, 66),
    (4235.4, 6522.7, 61),
    (4235.4, 1381.1, 55),
    (1596.7, 5577.5, 50),
    (1040.8, 4107.5, 70),
    (1096.9, 3674.1, 59),
    (4461.9, 6737.4, 65),
    (3100.3, 6907.0, 51),
    (1746.3, 2972.1, 54),
    (3734.3, 6528.3, 52),
    (4061.1, 6667.2, 66),
    (2238.4, 1251.0, 70),
    (6961.1, 3632.5, 65),
    (6155.9, 1473.0, 66),
    (3723.0, 988.0, 69),
    (1019.9, 4702.0, 51),
    (5864.8, 1421.1, 62),
    (2306.4, 1770.1, 65),
    (4370.2, 7194.1, 57),
    (5533.8, 1837.9, 59),
    (1114.2, 4343.6, 69),
    (5201.9, 6688.2, 57),
    (1390.5, 4257.7, 57),
    (3097.8, 931.9, 64),
    (6385.2, 5735.6, 57),
    (7095.4, 4212.5, 55),
    (3808.1, 1229.0, 51),
    (6640.7, 5865.7, 67),
    (6047.4, 6455.8, 65),
    (2782.7, 6211.3, 70),
    (1601.4, 5932.7, 63),
    (1376.5, 5049.1, 53),
    (4083.2, 747.9, 67),
    (1428.8, 2953.0, 60),
    (6246.3, 6143.0, 57),
    (3581.0, 1492.5, 69),
    (4719.9, 1455.9, 59),
    (6498.4, 4321.9, 67),
    (6629.1, 2530.8, 65),
    (5689.5, 1142.6, 69),
    (3612.3, 7201.9, 50),
    (5860.3, 5607.1, 50),
    (7096.9, 4636.7, 50),
    (1548.7, 4441.1, 64),
    (5984.8, 2176.9, 70),
    (3250.5, 6767.8, 63),
    (7007.3, 3403.6, 66),
    (5584.2, 6457.7, 59),
    (6049.6, 5464.7, 69),
    (6800.4, 3832.0, 53),
    (5172.3, 6249.5, 69),
    (1609.6, 1917.4, 52),
    (6907.4, 4503.3, 50),
    (4909.2, 6560.9, 52),
    (4917.2, 806.4, 52),
    (1119.7, 5062.1, 52),
    (2631.1, 1044.4, 65),
    (2456.7, 6836.4, 52),
    (6392.0, 3416.3, 61),
    (5434.3, 6288.1, 70),
    (6832.4, 5119.9, 67),
    (3869.0, 794.4, 68),
    (5441.2, 1060.6, 64),
    (6310.5, 2670.7, 66),
    (3773.3, 6746.5, 70),
    (1945.4, 2215.0, 56),
    (5697.8, 6010.4, 59),
    (5066.7, 1551.9, 54),
    (3321.1, 1637.6, 50),
    (1147.3, 5326.2, 62),
    (5212.5, 7120.9, 65),
    (6593.1, 3132.6, 60),
    (4878.6, 1192.2, 58),
    (4088.3, 1153.5, 59),
    (5507.8, 6783.0, 57),
    (1011.3, 3204.3, 56),
    (6831.8, 5346.1, 55),
    (1842.1, 6517.1, 65)
) AS v(x, y, q);

-- Plots v2's flip (V2.0, #459, the-capitol-plots-v2.md WP6): every zone takes
-- its v2 rings and layout, and the world grows from 13600 to 16600 so the
-- wilds keep their 2200 of depth. Urban runs out to r2160 (five rings of
-- blocks), the suburb to r3600 (lots of about 140 x 134), the rural ring to
-- r6100 (four main lanes, farms of about 35k), and the wilds to r8300.
--
-- 1. The centre moves from (6800, 6800) to (8300, 8300)
--    (capitol_protocol::rules::CENTER_X/Y), so every stored position moves by
--    +1500 on both axes and keeps its place relative to the tower, as RW1's
--    20261122_001_rural_widens.sql did. Every table with a position:
--      players               position, destination
--      resource_nodes        position
--      resource_node_seeds   position, while it exists: a fresh database has
--                            it until post-content's 20261116 plants and drops it
--      plots                 centre, bounds and corners (grid_x/y are indices)
--      stations              position
--    No table added since RW1 holds a position.
--
-- 2. Every plot on a suburban or rural lot of the old layout is renamed
--    `rw2_...`, so the plot sync can tell it from the new lots, which keep
--    the `sub_` and `farm_` schemes: most of the old ids name a different
--    lot now. Startup's sync deletes the unowned rows and lays the new lots,
--    and an owned one is released as a retired layout's
--    (`plot_sync::RETIRED_LOTS`), with its deed refunded and its stations and
--    their contents given back, as #422 and #423 did. A delete here would
--    destroy the stations' contents unbooked. A new prefix, because
--    `terrain_id` is unique and `oldsub_` and `oldfarm_` rows may still be
--    there. The urban lots (`urb_...`) and the guild halls keep theirs: the
--    first three rings of blocks are laid as they were, and every one of the
--    520 old urban lots is still there with its id and its shape, moved by
--    the shift alone. The two new rings past them are new ids.
--
-- 3. A saved `world_config` that still carries the old defaults takes the v2
--    ones, so it builds the default world; one an admin tuned keeps its own.
--    The land's absolute radii and `arterial_end_radius` go: the land follows
--    the rural edge (PV2, #454), and so do the arterials.
--
-- 4. Every player past r1800 goes to the spawn, on the south arterial at
--    r3950. The shift keeps each one at the same place relative to the centre,
--    but past r1800 every road, lot and stretch of water under them is new:
--    the urban grid's two new rings of blocks, the whole suburb, the farms and
--    the wild water. Inside r1800 the city is as it was.
--
-- 5. The seeded grass moves out with the rings: each node's radius remapped
--    zone by zone, the suburb's 1800 to 3000 onto 2160 to 3600 and the rural
--    ring's 3000 to 4600 onto 3600 to 6100, on the same bearing, then fitted
--    clear of the new roads, lots and water by `node_placement::place_nodes`.
--    The grass in sight of the spawn moves with it instead, 600 out along the
--    arterial, so a new player still arrives with grass in sight. The rows
--    come from the ignored test `print_grass_remap`: 156 nodes, 15 of them
--    fitted. The planned nodes are re-planned whole after the content sync,
--    in post-content/20261220_001_plots_v2_flip_nodes.sql.

-- 1. The shift.
UPDATE players SET
    position_x = position_x + 1500,
    position_y = position_y + 1500,
    destination_x = destination_x + 1500,
    destination_y = destination_y + 1500;

UPDATE resource_nodes SET
    position_x = position_x + 1500,
    position_y = position_y + 1500;

DO $$
BEGIN
    IF to_regclass('resource_node_seeds') IS NOT NULL THEN
        UPDATE resource_node_seeds SET
            position_x = position_x + 1500,
            position_y = position_y + 1500;
    END IF;
END $$;

-- `corners` is x0, y0, x1, y1, ...: every element moves by the same amount.
UPDATE plots SET
    world_x = world_x + 1500,
    world_y = world_y + 1500,
    bounds_min_x = bounds_min_x + 1500,
    bounds_min_y = bounds_min_y + 1500,
    bounds_max_x = bounds_max_x + 1500,
    bounds_max_y = bounds_max_y + 1500,
    corners = (SELECT array_agg(c + 1500 ORDER BY i) FROM unnest(corners) WITH ORDINALITY AS t(c, i));

UPDATE stations SET
    position_x = position_x + 1500,
    position_y = position_y + 1500;

-- 2. The old suburban and rural lots retire.
UPDATE plots SET terrain_id = 'rw2_' || terrain_id
WHERE terrain_id LIKE 'sub\_%' OR terrain_id LIKE 'farm\_%';

-- 3. The saved world, if any.
UPDATE world_config SET params = params - 'arterial_end_radius';

UPDATE world_config SET params = jsonb_set(params, '{land}', (params->'land') - 'foothill_radius' - 'range_radius')
WHERE jsonb_typeof(params->'land') = 'object';

UPDATE world_config SET params = jsonb_set(jsonb_set(params,
    '{urban,rings}', '5'),
    '{urban,outer_spokes}', '32')
WHERE params->'urban'->'rings' = '3'::jsonb
  AND coalesce(params->'urban'->'outer_spokes', '0'::jsonb) = '0'::jsonb;

UPDATE world_config SET params = jsonb_set(params, '{suburban,neighbourhood}',
    (params->'suburban'->'neighbourhood') || '{
        "lot_frontage": 140.0,
        "lot_width_min": 136.0,
        "lot_depth": 134.0,
        "lot_depth_min": 118.0,
        "lot_depth_max": 140.0,
        "cul_de_sac_max": 400.0,
        "pie_depth": 155.0,
        "warp_amplitude": 10.0
    }'::jsonb)
WHERE params->'suburban'->'neighbourhood'->'lot_frontage' = '100.0'::jsonb;

UPDATE world_config SET params = jsonb_set(params, '{rural}',
    (params->'rural') || '{
        "frontage": [132.0, 146.0],
        "field_gap": [40.0, 46.0],
        "depth": [230.0, 290.0],
        "min_frontage": 132.0,
        "min_depth": 185.0,
        "room": 12,
        "sway": 0.5,
        "lanes_follow": true,
        "mains": [[0.113, 0.147], [0.36, 0.394], [0.606, 0.64], [0.853, 0.887]]
    }'::jsonb)
WHERE params->'rural'->'frontage' = '[120.0, 160.0]'::jsonb;

-- 4. Everyone past r1800 to the spawn (rules::SPAWN_X/Y, 8300 and 12250),
--    scattered a little either side of the arterial as a new player is
--    (rules::SPAWN_SCATTER, 20), and stopped.
UPDATE players SET
    position_x = 8300 + (abs(hashtext(id::text)) % 41) - 20,
    position_y = 12250,
    destination_x = NULL,
    destination_y = NULL,
    action_state = 'idle',
    action_target_id = NULL,
    action_progress = 0,
    current_zone = 'rural'
WHERE (position_x - 8300) * (position_x - 8300) + (position_y - 8300) * (position_y - 8300) > 1800 * 1800;

-- 5. The seeded grass, matched on where the shift left it (to a twentieth,
--    for the sums' rounding), so a node moved by hand stays put, and running
--    this twice does nothing.
UPDATE resource_nodes AS n
SET position_x = v.nx, position_y = v.ny
FROM (VALUES
    (8180.3, 11681.5, 8180.3, 12281.5),
    (8406.4, 11770.0, 8406.4, 12370.0),
    (8181.3, 11609.4, 8181.3, 12209.4),
    (8248.4, 11784.6, 8248.4, 12384.6),
    (8455.8, 11688.7, 8455.8, 12288.7),
    (8377.3, 11597.7, 8363.9, 12154.8),
    (8598.4, 6091.1, 8658.1, 5649.3),
    (7093.8, 10012.8, 6887.5, 10427.4),
    (7654.7, 6594.2, 7525.6, 6253.0),
    (6141.4, 8624.5, 5635.1, 8648.7),
    (8385.1, 10575.4, 8402.1, 11030.5),
    (6858.6, 6698.8, 6570.3, 6378.6),
    (6549.4, 9176.7, 6209.2, 9257.5),
    (10223.8, 7181.8, 10608.6, 6958.2),
    (6440.6, 9687.6, 6110.1, 10001.3),
    (9444.1, 6497.0, 9672.9, 6136.4),
    (6717.5, 9793.3, 6401.0, 10092.0),
    (9860.9, 9534.5, 10173.1, 9781.4),
    (6464.3, 7330.6, 6097.2, 7136.7),
    (7236.3, 6749.2, 7023.6, 6439.0),
    (10122.9, 7092.7, 10487.5, 6851.2),
    (10571.4, 8886.5, 11025.7, 9003.8),
    (10097.0, 9080.4, 10456.4, 9236.5),
    (6514.5, 8717.1, 6157.4, 8800.5),
    (6130.5, 7831.2, 5696.6, 7737.4),
    (7829.1, 6477.7, 7734.9, 6113.2),
    (10286.3, 7661.7, 10683.6, 7534.0),
    (10068.8, 8865.1, 10422.6, 8978.1),
    (9948.7, 7060.5, 10278.4, 6812.6),
    (7418.1, 10357.7, 7241.7, 10769.2),
    (7201.7, 9842.9, 6982.0, 10151.5),
    (9534.8, 6823.7, 9781.8, 6528.4),
    (8752.8, 6421.3, 8847.1, 6005.8),
    (8782.2, 6018.2, 8878.6, 5561.8),
    (9242.5, 6687.9, 9431.0, 6365.5),
    (9242.4, 10022.7, 9430.9, 10367.2),
    (6391.1, 7147.1, 6009.3, 6916.5),
    (10243.9, 8840.2, 10632.7, 8948.2),
    (7940.6, 6307.1, 7868.7, 5908.5),
    (6958.0, 9523.1, 6689.6, 9767.7),
    (6667.0, 9562.5, 6340.4, 9815.0),
    (6225.5, 7621.2, 5810.6, 7485.4),
    (6913.6, 7019.3, 6636.3, 6763.2),
    (6481.5, 6812.7, 6117.8, 6515.2),
    (8197.1, 6240.1, 8176.5, 5828.1),
    (6767.9, 6881.1, 6461.5, 6597.3),
    (8629.1, 10101.7, 8694.9, 10462.0),
    (7804.4, 6242.1, 7705.3, 5830.5),
    (6498.6, 9036.0, 6138.3, 9183.2),
    (6379.8, 9111.9, 5995.8, 9274.3),
    (10273.2, 8234.2, 10667.8, 8221.0),
    (9795.3, 9316.4, 10094.4, 9519.7),
    (10415.0, 7555.3, 10838.0, 7406.4),
    (9822.9, 9720.2, 10127.5, 10004.2),
    (10570.9, 8361.3, 11025.1, 8373.6),
    (6380.2, 7792.1, 5996.2, 7690.5),
    (6433.5, 8122.1, 6060.2, 8086.5),
    (9706.3, 10102.3, 9987.6, 10462.8),
    (10598.6, 7911.1, 11058.3, 7833.3),
    (6254.7, 8545.6, 5845.6, 8594.7),
    (10336.7, 9217.6, 10744.0, 9401.1),
    (7315.7, 10061.3, 7058.4, 10315.7),
    (6231.2, 8570.8, 5817.4, 8625.0),
    (6199.2, 7440.2, 5779.0, 7268.2),
    (10315.9, 7823.3, 10719.1, 7728.0),
    (9162.1, 6304.1, 9334.5, 5904.9),
    (6808.5, 5504.7, 6481.5, 4891.8),
    (5074.7, 9316.5, 4297.7, 9561.4),
    (10104.6, 11055.0, 10523.8, 11695.0),
    (7512.1, 5415.3, 7354.5, 4838.4),
    (7044.2, 5554.2, 6790.1, 4998.7),
    (5758.0, 7818.7, 5249.6, 7722.4),
    (10228.7, 6159.5, 10614.4, 5731.4),
    (10804.7, 8108.7, 11305.6, 8070.4),
    (5212.5, 7265.7, 4507.0, 7029.3),
    (6574.4, 10668.9, 6229.3, 11142.7),
    (6315.4, 9847.8, 5785.0, 10177.3),
    (11234.3, 7454.0, 11839.9, 7279.4),
    (10652.4, 9277.3, 11122.9, 9472.8),
    (8535.4, 10822.7, 8582.5, 11327.2),
    (8415.3, 5568.0, 8438.4, 5021.6),
    (5931.0, 9920.5, 5457.2, 10244.6),
    (5340.8, 8407.5, 4749.0, 8429.0),
    (5396.9, 7974.1, 4816.3, 7908.9),
    (8761.9, 11037.4, 8854.3, 11584.9),
    (7400.3, 11207.0, 7215.7, 11803.3),
    (6046.3, 7272.1, 5595.6, 7066.5),
    (8034.3, 10828.3, 7981.2, 11334.0),
    (8361.1, 10967.2, 8373.3, 11500.6),
    (6538.4, 5551.0, 6134.3, 4920.3),
    (11261.1, 7932.5, 11853.3, 7859.0),
    (10455.9, 5773.0, 10962.8, 5178.9),
    (8023.0, 5288.0, 7966.8, 4676.7),
    (5319.9, 9002.0, 4702.1, 9147.5),
    (10164.8, 5721.1, 10583.6, 5133.0),
    (6606.4, 6070.1, 6267.7, 5624.1),
    (8670.2, 11494.1, 8753.2, 12210.5),
    (9890.5, 6118.4, 10150.9, 5565.6),
    (5414.2, 8643.6, 4837.0, 8712.3),
    (9501.9, 10988.2, 9742.3, 11525.8),
    (5699.0, 8491.1, 5178.8, 8529.3),
    (7397.8, 5231.9, 7197.1, 4549.4),
    (10685.2, 10035.6, 11162.2, 10382.7),
    (11395.4, 8512.5, 12051.6, 8557.5),
    (8108.1, 5529.0, 8069.7, 4974.8),
    (10940.7, 10165.7, 11537.9, 10587.6),
    (10347.4, 10755.8, 10802.7, 11301.9),
    (7082.7, 10511.3, 6849.2, 11083.2),
    (5901.4, 10232.7, 5399.0, 10637.5),
    (5676.5, 9349.1, 5151.8, 9558.9),
    (8383.2, 5047.9, 8402.2, 4305.7),
    (5728.8, 7253.0, 5214.6, 7043.6),
    (10546.3, 10443.0, 11023.0, 10897.8),
    (7881.0, 5792.5, 7797.2, 5291.0),
    (9202.7, 5903.5, 9383.2, 5424.2),
    (10798.4, 8621.9, 11298.1, 8686.3),
    (10929.1, 6830.8, 11458.6, 6534.9),
    (9989.5, 5442.6, 10386.3, 4771.4),
    (7912.3, 11501.9, 7875.1, 12256.2),
    (10240.1, 9912.1, 10628.1, 10234.5),
    (11396.9, 8936.7, 12098.7, 9075.8),
    (5848.7, 8741.1, 5358.4, 8829.3),
    (10284.8, 6476.9, 10681.8, 6112.3),
    (7368.2, 11150.1, 7181.8, 11720.1),
    (11307.3, 7703.6, 11932.2, 7579.7),
    (9884.2, 10757.7, 10201.0, 11249.2),
    (10349.6, 9764.7, 10759.5, 10057.6),
    (11089.5, 8093.5, 11647.4, 8052.2),
    (9472.3, 10549.5, 9706.8, 10999.4),
    (5909.6, 6217.4, 5385.0, 5760.3),
    (11207.4, 8803.3, 11788.9, 8904.0),
    (9281.5, 10758.9, 9477.8, 11250.7),
    (9217.2, 5106.4, 9432.9, 4355.2),
    (5419.7, 9362.1, 4819.9, 9583.3),
    (6931.1, 5344.4, 6618.1, 4668.7),
    (6756.7, 11136.4, 6408.4, 11776.6),
    (10692.0, 7716.3, 11170.4, 7599.6),
    (9734.3, 10588.1, 10021.2, 11045.7),
    (11132.4, 9419.9, 11714.3, 9650.0),
    (8169.0, 5094.4, 8139.7, 4377.8),
    (9741.2, 5360.6, 10073.1, 4683.6),
    (10610.5, 6970.7, 11072.6, 6704.8),
    (8073.3, 11046.5, 8028.0, 11595.8),
    (6245.4, 6515.0, 5834.5, 6158.0),
    (9997.8, 10310.4, 10238.6, 10644.4),
    (9366.7, 5851.9, 9580.0, 5362.3),
    (7621.1, 5937.6, 7485.3, 5465.1),
    (5447.3, 9626.2, 4828.8, 9913.7),
    (9512.5, 11420.9, 9800.7, 12162.7),
    (10893.1, 7432.6, 11411.7, 7259.1),
    (9178.6, 5492.2, 9354.3, 4930.6),
    (8388.3, 5453.5, 8406.0, 4884.2),
    (9807.8, 11083.0, 10146.5, 11653.2),
    (5311.3, 7504.3, 4681.0, 7336.5),
    (11131.8, 9646.1, 11742.5, 9936.4),
    (6142.1, 10817.1, 5636.1, 11407.3)
) AS v(x, y, nx, ny)
WHERE n.resource_type = 'grass' AND abs(n.position_x - v.x) < 0.05 AND abs(n.position_y - v.y) < 0.05;

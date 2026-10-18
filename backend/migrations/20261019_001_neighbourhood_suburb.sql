-- #112: the Neighbourhood layout is the default suburb (suburb v2, #103).
-- Its streets and lots replace the Legacy suburb's, so:
--
-- 1. Owned Legacy suburban plots are released, with the deed refunded and
--    their stations removed, as an admin's forced commit does
--    (`engine::plot_sync`, `OnOwned::Release`). Startup's sync keeps an owned
--    plot whose lot is gone, so without this they would stand on top of the
--    new lots. Once released, that sync deletes them. Legacy lot ids start
--    with their street (collector_, culdesac_, loop_ and so on); the new ones
--    start with sub_. A world whose saved params still name the Legacy layout
--    keeps its suburb, so its plots are left alone.
--
-- 2. 19 grass nodes the new streets run over move to the nearest spot in
--    their zone that is 30 clear of every road, off every lot and 30 from
--    other nodes, as `engine::node_placement::place_nodes` would. Startup
--    would make these moves anyway, but this way the default world moves no
--    seeded node, which `models::resource::tests` check. The rows come from
--    the ignored test `print_node_moves`. None was stuck.
--
-- Named 20261019: sqlx takes the version from the digits before the first
-- underscore, and 20261018 is taken.

DO $$
DECLARE
    plot RECORD;
    before BIGINT;
BEGIN
    IF EXISTS (SELECT 1 FROM world_config WHERE params->'suburban'->>'layout' = 'legacy') THEN
        RETURN;
    END IF;
    FOR plot IN
        SELECT id, owner_id, assessed_value FROM plots
        WHERE zone_id = 'suburban' AND owner_id IS NOT NULL
          AND terrain_id IS NOT NULL AND terrain_id NOT LIKE 'sub\_%'
    LOOP
        DELETE FROM stations WHERE plot_id = plot.id;
        UPDATE plots SET owner_id = NULL, claimed_at = NULL, assessed_value = 0,
               tax_owed = 0, last_tax_paid = NULL, station_count = 0
        WHERE id = plot.id;
        UPDATE plot_permissions SET entry_level = 'public', visibility_level = 'public',
               station_use_level = 'owner', modify_level = 'owner'
        WHERE plot_id = plot.id;
        IF plot.assessed_value > 0 THEN
            SELECT strand_balance INTO before FROM players WHERE id = plot.owner_id FOR UPDATE;
            UPDATE players SET strand_balance = before + plot.assessed_value WHERE id = plot.owner_id;
            INSERT INTO currency_transactions
                (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
            VALUES (plot.owner_id, plot.assessed_value, before, before + plot.assessed_value, 'deed_refund',
                    plot.id, 'Deed refunded: the suburb was rebuilt as neighbourhoods');
        END IF;
    END LOOP;
END $$;

-- Matched on the previous position, so a node moved by hand stays put, and
-- running this twice does nothing.
UPDATE resource_nodes AS n
SET position_x = v.nx, position_y = v.ny
FROM (VALUES
    (3272.0, 2193.9, 3354.7, 2294.2),
    (1811.4, 4324.5, 1841.4, 4324.5),
    (4073.9, 6160.9, 4085.1, 6275.4),
    (5198.2, 2187.2, 5144.1, 2197.0),
    (5918.4, 3420.8, 5986.3, 3361.7),
    (5595.1, 2631.2, 5648.7, 2760.5),
    (5059.1, 5636.7, 4942.4, 5722.7),
    (2134.5, 4646.6, 2198.6, 4736.0),
    (6050.1, 3172.8, 6115.0, 3255.3),
    (6145.9, 4061.3, 6270.9, 4061.3),
    (2034.1, 4176.8, 1954.7, 4245.6),
    (1976.1, 4268.3, 1931.2, 4270.8),
    (1978.5, 5595.1, 2015.4, 5547.8),
    (4235.4, 1381.1, 4115.3, 1268.0),
    (5533.8, 1837.9, 5590.5, 1818.4),
    (1343.3, 4285.9, 1399.0, 4191.1),
    (4719.9, 1455.9, 4902.7, 1603.5),
    (3250.5, 6767.8, 3068.2, 6850.1),
    (4909.2, 6560.9, 4981.5, 6458.9)
) AS v(x, y, nx, ny)
WHERE n.resource_type = 'grass' AND n.position_x = v.x AND n.position_y = v.y;

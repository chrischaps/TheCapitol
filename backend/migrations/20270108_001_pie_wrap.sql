-- #470: pies on a stem with no lots near its bulb wrap on round it.
--
-- A cul-de-sac whose stem has no lot run ending near its bulb gave its pies
-- only a right angle either side of the stem's line, 180° between them, and
-- three pies shared that as slivers about 25 across the front. The pies now
-- wrap on to 135° on such a side, short of the back of its nearest stem lot
-- (`lots::Street::pies`). In the default world five bulbs in sectors 5 and 6
-- change: of the old lots, 1 pie is gone and 11 pies and 8 row lots beside
-- them keep their id with a new shape, and 4 pies are new. Nothing outside
-- the suburb moves.
--
-- Every plot on one of those 20 is renamed `rewrap_…`, so the plot sync can
-- tell it from the new lot of the same id: startup's sync deletes the
-- unowned rows and lays the new lots, and an owned one is released as a
-- retired layout's (`plot_sync::RETIRED_LOTS`), with the deed refunded and
-- its stations and their contents given back, as #467 released the farms.
--
-- The suburb's planned nodes that the new lots move are re-planned after the
-- content sync, in post-content/20270108_001_pie_wrap_nodes.sql. No seeded
-- grass moves (`the_default_terrain_moves_no_seeded_node`).

UPDATE plots SET terrain_id = 'rewrap_' || terrain_id WHERE terrain_id IN (
    'sub_5_0_19_B_0',
    'sub_5_0_19_B_1',
    'sub_5_0_21_L_17',
    'sub_5_0_21_R_0',
    'sub_6_0_11_B_0',
    'sub_6_0_11_B_2',
    'sub_6_0_12_B_1',
    'sub_6_0_12_B_2',
    'sub_6_0_13_B_0',
    'sub_6_0_16_L_3',
    'sub_6_0_16_R_0',
    'sub_6_0_17_L_0',
    'sub_6_0_17_R_2',
    'sub_6_0_19_L_0',
    'sub_6_0_19_L_1',
    'sub_6_0_8_B_0',
    'sub_6_0_8_B_1',
    'sub_6_0_8_B_2',
    'sub_6_0_9_B_0',
    'sub_6_0_9_B_1'
);

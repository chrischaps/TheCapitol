-- #222: the wilds' water runs on into the rural ring, and some of E2.3's
-- planned nodes (woods, branches and outcrops on the rural plain) stand in it
-- or within 15 of it.
--
-- 40 of them move to the nearest spot in their zone that is 30 clear of every
-- road and the water, off every lot and 30 from other nodes, as
-- `engine::node_placement::place_nodes` would. Startup would make these moves
-- anyway, but this way the default world moves no planned node, which
-- `models::resource::tests` check. The rows come from the ignored test
-- `print_planned_node_moves`. None was stuck, and no seeded grass node needed
-- a move.
--
-- Both the staged seeds and any already planted move: the plan's ids are the
-- nodes' ids (E2.3, #207).
CREATE TEMP TABLE wild_water_moves (id UUID PRIMARY KEY, x DOUBLE PRECISION NOT NULL, y DOUBLE PRECISION NOT NULL);

INSERT INTO wild_water_moves (id, x, y)
VALUES
    ('09f4c9d8-08e9-4bae-beb5-070c63af36b5', 2625.5, 4906.1),
    ('15b7aff1-aa47-4471-9647-cd4d34210b7c', 3582.1, 8845.1),
    ('1d8230eb-cab7-445a-8f27-149e25ac3223', 2464.7, 6581.5),
    ('284aa404-71d1-40c7-bed7-e56c2f53a89e', 8726.5, 3695.4),
    ('2d28d7e6-d082-4ab9-a648-e1df6e8dd394', 2381.4, 4848.5),
    ('2dd2ef25-7c4f-4a6c-b7df-9249803e21ac', 2499.7, 6727.0),
    ('3386e68f-0ed6-4da3-8b1a-aded16e5ab43', 3623.0, 8906.7),
    ('36d0828e-957e-4e6e-8702-bc5acc683ce8', 3449.5, 8032.2),
    ('3a9cdf9b-db87-423e-b904-5b0774182e86', 6507.2, 9312.7),
    ('46395ecd-424c-4698-98ca-6055e2221df7', 6420.2, 9492.9),
    ('56a497c3-2aa5-4f4b-b6d9-4b06b034dfe5', 2411.7, 4851.1),
    ('590575ea-5f93-453d-aab7-7b567418503f', 6503.4, 9276.4),
    ('5bb9c9ac-a2e8-47f1-b4f0-4ea03593e95f', 2528.4, 6684.7),
    ('5bf7f0d9-24c7-4958-a9be-9db9c90c46af', 2347.7, 6960.4),
    ('5c1114f7-ca65-471c-b334-4ad1b6a3b726', 3417.7, 8039.4),
    ('5f5385c2-de54-4682-add7-ce1eb09bf7af', 2391.5, 6698.8),
    ('64b1aec7-f852-4b2d-8db0-3eba9518376e', 2581.4, 6634.1),
    ('69903f90-b044-44a2-b799-5ec21587a905', 6559.3, 9367.3),
    ('6f2fd1e6-bd1d-4509-b29e-759ffb70c391', 2391.0, 4973.4),
    ('7adf6e3c-f948-40f3-9e60-97e06b629d09', 2576.9, 4750.4),
    ('7c6b9845-40a9-4229-a71d-0cf91bdb72fc', 2359.0, 4975.6),
    ('8365dd8d-fc67-4ac4-b3bc-d19a9fbe6b2e', 2494.8, 6578.9),
    ('853e571a-794e-4f5b-b7e9-bae235bdc0cd', 8825.8, 3594.7),
    ('9b70ff6f-c529-4b42-80f4-4a27108b5c46', 2554.3, 6702.6),
    ('9dd6de5d-9810-4cbc-8988-53603e3984b4', 6581.9, 9391.3),
    ('9e5b9708-e6e2-4c65-9fba-4841a52d893e', 6585.0, 9349.0),
    ('a0442884-caea-41ed-bf9b-5456c6278e9d', 2532.2, 6723.7),
    ('a2cd951b-5aa2-42bb-92c1-840911f2a498', 6430.9, 9248.6),
    ('ad0f71af-592c-47b1-b6c8-bc174b5bb79b', 6461.6, 9256.4),
    ('c054f239-af1d-4afe-b3d3-e6eb982d4539', 2458.6, 6766.8),
    ('c0b25c40-9881-4f45-8b56-d3cc77b1e2bd', 8755.6, 3567.9),
    ('c6bdae8b-7b3f-4eee-9618-0eeee77260dd', 8849.4, 8427.4),
    ('c779fac8-a1d6-4975-a307-830bd0fdc5d9', 8900.6, 8279.2),
    ('ccfa9508-3872-4dec-8c1b-1036be816f8e', 2340.4, 6897.2),
    ('e929102d-6982-4fa4-8709-5aea6c481288', 2611.3, 6642.1),
    ('f00cfbe5-1e48-479e-afc3-3c6b4aa72f79', 2558.5, 6656.4),
    ('f1976553-4f1e-4267-9236-75e7f4455773', 8786.8, 3423.1),
    ('f66b4b43-20ed-432d-b628-138f6722a170', 6593.0, 9420.9),
    ('f9bd139c-0841-4bec-85fa-04d19269f0ab', 8747.5, 3723.9),
    ('fe4c3be7-b026-4635-94b9-a868cc582734', 6387.8, 9481.5);

UPDATE resource_node_seeds AS s SET position_x = m.x, position_y = m.y
FROM wild_water_moves AS m WHERE s.id = m.id;

UPDATE resource_nodes AS n SET position_x = m.x, position_y = m.y
FROM wild_water_moves AS m WHERE n.id = m.id;

DROP TABLE wild_water_moves;

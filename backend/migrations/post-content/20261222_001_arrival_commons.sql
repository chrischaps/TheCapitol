-- Arrival's commons (O1.1, #443): what the road in's first chapter gathers,
-- grown within a short walk of the spawn on the south arterial.
--
-- Chapter 1 gathers grass, fallen branches and stone by hand, and fells a
-- tree with the stone axe it makes. The rural ring grows those thinly: on
-- the default world the nearest tree stood 300 from the spawn and the
-- nearest outcrop 466, with only 2 outcrops inside 600. So the planner now
-- adds a commons round the spawn, 180 of it at most (`node_spawn::arrival`):
-- a copse with branches under it, three knots of outcrops, and grass
-- between, all on open ground off every lot, enough for three newcomers at
-- once to walk the chapter with none waiting on a regrowth.
--
-- The commons is placed after every other node, so the rest of the plan is
-- as the flip's 20261220_001_plots_v2_flip_nodes.sql wrote it, and this only
-- adds. The trees, branches and outcrops are planned types, so a regenerate
-- re-plans them with the rest. The grass is grass: like the seeded grass, a
-- regenerate leaves it be, and startup's fit moves it if a road runs over it.
--
-- The rows come from the ignored test `print_new_nodes`, and
-- `the_migration_writes_the_default_plan` checks they still match the plan.
--
--   fallen_branches in rural: 6
--   grass in rural: 19
--   rock_outcrop in rural: 12
--   tree in rural: 6

INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality)
VALUES
    ('a8fa6881-088c-4fce-9a80-5c4c025025fe', 'tree', 8189.9, 12173.5, 38),
    ('da4464b2-a425-40f0-820a-03ac2c871b6b', 'tree', 8209.3, 12158.1, 66),
    ('09b728ff-2706-4ebb-91e5-0c5d9b33d118', 'tree', 8215.0, 12142.9, 67),
    ('3032e564-3a03-4f8f-82b4-905cb1722c17', 'tree', 8188.3, 12152.0, 38),
    ('1fad92af-cd98-45f2-a0ec-2aa0ef5a576b', 'tree', 8185.5, 12187.6, 49),
    ('dce59f8d-b9f0-47f9-9192-6ecd25a1da10', 'tree', 8209.4, 12170.5, 51),
    ('8c79e27d-4d75-4892-92cc-090d71e0918e', 'fallen_branches', 8223.6, 12152.3, 38),
    ('67e1318c-ba3e-48df-a4be-e924392b354a', 'fallen_branches', 8143.7, 12190.0, 53),
    ('61189e16-f9e2-41c6-9a10-013be203c7ff', 'fallen_branches', 8226.2, 12166.9, 41),
    ('35bf1dbb-30ca-4e2f-bc92-b24c27ddf51c', 'fallen_branches', 8179.0, 12136.7, 38),
    ('5ff456b5-18cc-489e-bc5f-9b5ad070560f', 'fallen_branches', 8154.1, 12167.7, 63),
    ('1265f848-cd0f-45b4-bb6f-cd29e376dad3', 'fallen_branches', 8216.6, 12189.3, 51),
    ('979ab5d9-5385-43b5-bf7e-f7553831f198', 'rock_outcrop', 8366.5, 12102.3, 35),
    ('7029b0be-4240-4d34-a129-dd45863aa91b', 'rock_outcrop', 8365.5, 12088.3, 45),
    ('89ff6ded-0131-46a1-a644-680a7b7100cc', 'rock_outcrop', 8383.5, 12121.3, 61),
    ('84f0f6c3-2d19-4726-ba03-85b1300f0d4f', 'rock_outcrop', 8378.3, 12098.6, 75),
    ('ba1b86cd-991e-4e30-9772-b148bb52677f', 'rock_outcrop', 8363.6, 12364.9, 65),
    ('67708406-7c1c-4a76-91c1-461420c5a685', 'rock_outcrop', 8367.0, 12376.6, 67),
    ('5b8ed121-f367-408d-8b41-6376e4c6e628', 'rock_outcrop', 8379.4, 12363.8, 33),
    ('0d41c037-6e28-4ad3-919f-c5425290c25c', 'rock_outcrop', 8388.8, 12373.5, 46),
    ('8e5bd682-cd23-4816-9b89-c63a816bd7f0', 'rock_outcrop', 8238.5, 12163.6, 40),
    ('a9ee3f9d-652a-4c45-b154-4e78d8add27b', 'rock_outcrop', 8232.2, 12181.6, 48),
    ('137ee0a5-72a0-4ca9-ac85-b2523e472a93', 'rock_outcrop', 8238.7, 12144.3, 47),
    ('135c5577-3af2-4ea0-96a1-de9cdbff03fb', 'rock_outcrop', 8238.6, 12193.0, 61),
    ('aeb768f7-ac88-4ca5-8665-9c95bec66ba8', 'grass', 8172.0, 12153.3, 60),
    ('19d94ff1-f502-4242-872b-45838a092236', 'grass', 8239.2, 12392.8, 50),
    ('ce144afc-e8eb-4024-9914-332f90193727', 'grass', 8405.1, 12116.9, 58),
    ('059d0a6a-16a4-4614-9001-bd7c92c2015e', 'grass', 8155.6, 12194.1, 46),
    ('fa19f56d-0a12-43d3-9293-21728124a772', 'grass', 8398.0, 12132.5, 59),
    ('74a2b71f-4fe5-4185-9776-c66bdb7e6f35', 'grass', 8370.4, 12141.2, 70),
    ('47d5b3f5-0b46-49ad-8445-34d2fa175899', 'grass', 8410.8, 12130.0, 41),
    ('0ed78bf6-c440-4d42-9f90-e5c5876174bb', 'grass', 8221.1, 12105.7, 53),
    ('1b1df10d-9baf-4e48-800c-9efd1e3a5114', 'grass', 8221.8, 12393.0, 70),
    ('d63cd09d-8f8e-4258-905e-52a8d664e3dd', 'grass', 8423.9, 12133.2, 56),
    ('fc5c30d2-7631-4854-b8b9-b758b49644c7', 'grass', 8194.1, 12123.3, 49),
    ('b4d96485-0c2b-4508-8d4f-923185a92e20', 'grass', 8393.4, 12109.0, 48),
    ('8976b851-8c34-48fd-be17-e0415b9b8faa', 'grass', 8205.9, 12119.0, 69),
    ('f48dc1e1-249d-4457-b7a0-9f5af5449331', 'grass', 8170.2, 12184.2, 41),
    ('cab02bb8-7f43-4c2d-a260-686c68fa311a', 'grass', 8237.2, 12124.9, 44),
    ('1aa3a7a2-a812-4c98-8357-c008e3bce7da', 'grass', 8199.5, 12204.9, 68),
    ('56f371dd-f125-4d26-a90e-8843afed5cac', 'grass', 8401.1, 12147.9, 70),
    ('d486d328-45da-4e08-aaae-f8dff97b7c96', 'grass', 8436.1, 12135.6, 58),
    ('4cc5484f-4a93-4b5e-b2cd-bb916933613d', 'grass', 8360.6, 12124.9, 47)
ON CONFLICT (id) DO NOTHING;

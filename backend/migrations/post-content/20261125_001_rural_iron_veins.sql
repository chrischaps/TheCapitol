-- #210: four times the rural iron veins, 19 to 76 (Rural density 0.5 to 2.0).
--
-- A vein gives one pull and grows back in 6000 ticks, so the ring's veins
-- cap the town's ore: 19 of them gave at most about 115 pulls a game hour,
-- half of them failing with a stone pick, to C1.7b's 29 ore gatherers
-- (#306: ore at 267 s a unit against D5's 90, and "found no iron_vein"
-- 12,076 times). The wilds' 630 veins stay the rich source, higher in
-- quality and up in the ranges.
--
-- Only the rural veins move: the plan's other nodes keep their ids and
-- places. The spawn's stream hands the veins new ids, so RW1's 19 go and
-- the 76 come in. A vein half gathered starts over; nothing a player owns
-- is a node.
--
-- The rows come from the ignored test `print_new_nodes`, and
-- `the_migration_writes_the_default_plan` checks the plan still matches.
DELETE FROM resource_nodes
WHERE resource_type = 'iron_vein' AND id IN (
    '04a8ec37-4152-4a19-afd4-329f485319f0',
    '1ee8f6f9-3b1f-4070-ad8e-f4f6d16e60a8',
    '306f8ee6-19cf-4166-8be6-2752403bbdcf',
    '399a659c-cb16-411d-abc6-c234686a3a4a',
    '3b5e6f28-8458-4d95-a320-9e50b421d655',
    '3d86d329-c653-4665-91d5-488eb3776444',
    '810fb958-698c-4b6b-8407-a339785c1031',
    '87da408e-ee45-4203-b9d3-4d879322da8b',
    '91f11f09-69d9-4843-9b13-445dd6e461f5',
    'a3bfb658-eb71-49ee-80d9-4603db6cb211',
    'a4e133f2-2edb-4403-ab8f-7789bc142a0f',
    'afd50b27-a46c-40f3-8b16-10ba42a33e59',
    'b7574e2a-3335-41ba-8482-7b0e253b1de0',
    'c79f0a4d-d446-48e6-8ecc-c8b9e31c20b1',
    'c9553f07-7818-4c0f-84a4-42935573ff72',
    'ce601588-fdd7-43b1-af62-816ece2828b0',
    'df8d81d1-16a7-447d-a8ba-422284fb731c',
    'e37d80ec-073e-4fdb-b382-aa0cc5390e0f',
    'f977355d-086e-420c-8037-35fd5b2407bf'
);

INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality)
VALUES
    ('00652942-09a1-446c-a631-3d7cac953317', 'iron_vein', 2822.3, 5591.6, 89),
    ('03ced25a-ef98-48a1-bfaf-bd82bc107abf', 'iron_vein', 9289.7, 2995.3, 28),
    ('04a8ec37-4152-4a19-afd4-329f485319f0', 'iron_vein', 8501.0, 3877.7, 51),
    ('06808207-8a3e-4f0b-a7cd-9e820b65634f', 'iron_vein', 9259.5, 2944.0, 36),
    ('09eb7939-a88c-4dcc-8adf-b1d2d1186f92', 'iron_vein', 10347.5, 5755.6, 58),
    ('0a982c01-2271-432b-8b40-b010e0095b0d', 'iron_vein', 10142.9, 4639.7, 75),
    ('0bce031f-1af7-4474-8a4d-bf347b25bd2b', 'iron_vein', 5940.2, 2548.7, 61),
    ('1698a641-1af4-4f95-88df-d0cb469e487b', 'iron_vein', 9284.1, 2962.9, 67),
    ('1b876ad2-32bb-4196-9284-4fccc3495363', 'iron_vein', 6166.3, 3736.5, 42),
    ('1d42c5e0-59cc-4291-b485-b06b328c6623', 'iron_vein', 9313.0, 2981.5, 49),
    ('1d703e40-84ce-4181-ad9a-2d56d7943cb3', 'iron_vein', 8024.1, 2682.4, 82),
    ('1edf9c05-8ad2-4f23-92db-e40f5fbbab7d', 'iron_vein', 9423.7, 9933.1, 68),
    ('1ee8f6f9-3b1f-4070-ad8e-f4f6d16e60a8', 'iron_vein', 6005.5, 3215.8, 43),
    ('23cd2130-c2e9-4bef-af6e-d9521f1597ad', 'iron_vein', 2498.1, 5820.2, 51),
    ('306f8ee6-19cf-4166-8be6-2752403bbdcf', 'iron_vein', 5983.3, 3212.2, 73),
    ('31d462b2-ce6f-43c7-a764-fd316f91cbd9', 'iron_vein', 6198.0, 3761.5, 46),
    ('37635ab3-405f-4cd3-ba13-c69aeeec67ff', 'iron_vein', 9276.0, 2977.9, 89),
    ('38df1e6a-82af-4065-88d2-94f88ceb2412', 'iron_vein', 5972.9, 2550.5, 65),
    ('399a659c-cb16-411d-abc6-c234686a3a4a', 'iron_vein', 8529.7, 3833.9, 54),
    ('3b5e6f28-8458-4d95-a320-9e50b421d655', 'iron_vein', 8488.7, 3864.8, 79),
    ('3d40dcf1-4668-4879-897a-4efb05716594', 'iron_vein', 6415.6, 2878.2, 63),
    ('3d86d329-c653-4665-91d5-488eb3776444', 'iron_vein', 3470.5, 6431.1, 73),
    ('47b4a326-d087-4112-95f4-baa5989612db', 'iron_vein', 3422.2, 6002.5, 84),
    ('58cddb56-7522-4c15-a22b-ce1add9a3d89', 'iron_vein', 3448.5, 6952.2, 44),
    ('5e0a7d42-1a9a-4cf8-a2cc-3a6d0e3f3c8d', 'iron_vein', 10602.2, 7541.9, 39),
    ('5fef2467-805d-4099-b50d-d43b5c7776f7', 'iron_vein', 3761.3, 5880.7, 83),
    ('602382cd-4745-46cf-b11e-291f4f222437', 'iron_vein', 10218.1, 4306.0, 43),
    ('6501d9c0-887f-4c96-88b0-b6ae06d465d3', 'iron_vein', 6184.2, 3763.6, 61),
    ('684fee52-c075-48e5-b070-f8f3986597e8', 'iron_vein', 2500.5, 5863.6, 58),
    ('77591c3a-d26e-4145-94de-cfd3af3f85a3', 'iron_vein', 8058.4, 2687.0, 58),
    ('7a4d1f05-c2e3-4d31-b7a1-b84cd7716cf1', 'iron_vein', 3397.4, 6025.3, 73),
    ('810fb958-698c-4b6b-8407-a339785c1031', 'iron_vein', 8506.9, 3834.1, 74),
    ('84f44478-c044-47e5-8696-4873a03c982d', 'iron_vein', 3749.9, 5344.5, 82),
    ('85118f6f-ad7c-4246-abad-f440a17e47fe', 'iron_vein', 9534.8, 3128.5, 80),
    ('85fe897e-9c5a-47b2-9941-903b3321bf16', 'iron_vein', 9437.2, 9960.7, 34),
    ('87da408e-ee45-4203-b9d3-4d879322da8b', 'iron_vein', 3474.1, 6462.6, 77),
    ('91f11f09-69d9-4843-9b13-445dd6e461f5', 'iron_vein', 9286.3, 8849.1, 74),
    ('9653a812-2ecd-4f43-aed6-7d73b8ecfdf1', 'iron_vein', 10694.3, 7342.1, 37),
    ('9a1a909b-4549-41df-9183-c6830b1873f7', 'iron_vein', 10242.3, 4300.9, 53),
    ('9b1f1bc1-99c5-4646-8260-d5daa1c5a00d', 'iron_vein', 3438.7, 6038.6, 32),
    ('9e1f8dc3-54f4-4949-ace1-18025751835c', 'iron_vein', 3776.2, 5893.9, 63),
    ('9f6560e0-e0df-4864-aa78-1abdabedae30', 'iron_vein', 9401.9, 9933.6, 67),
    ('a3bfb658-eb71-49ee-80d9-4603db6cb211', 'iron_vein', 10368.5, 5746.6, 49),
    ('a42382a1-0f7c-4ff5-87ef-7c32ce758ff7', 'iron_vein', 10634.3, 7544.1, 50),
    ('a4b911e0-5031-44e7-a0d5-3d35035783f2', 'iron_vein', 10200.5, 4321.1, 67),
    ('a4e133f2-2edb-4403-ab8f-7789bc142a0f', 'iron_vein', 9281.4, 8868.2, 68),
    ('aa48a789-5e33-4733-bc00-9721f254866f', 'iron_vein', 10336.9, 5735.5, 79),
    ('afd50b27-a46c-40f3-8b16-10ba42a33e59', 'iron_vein', 9306.5, 8870.2, 27),
    ('b7574e2a-3335-41ba-8482-7b0e253b1de0', 'iron_vein', 9294.9, 8865.9, 25),
    ('b8b7e830-2911-411f-93be-a80d43452b68', 'iron_vein', 11053.0, 8498.7, 51),
    ('b8e8414e-8e59-4bc7-b1dd-afaa349ae9dd', 'iron_vein', 9419.0, 9949.9, 89),
    ('bb41d2d2-9715-410e-9abc-4a7ab34b3076', 'iron_vein', 10169.2, 4637.1, 71),
    ('c43c436e-4b03-4fad-8258-c9e320e0c997', 'iron_vein', 3425.5, 6014.2, 63),
    ('c611ea39-204e-4a81-a700-ae76c6883e27', 'iron_vein', 3481.0, 6962.4, 51),
    ('c72bdb5e-8541-4c29-90e3-4e4e3956db15', 'iron_vein', 5943.1, 2525.5, 26),
    ('c79f0a4d-d446-48e6-8ecc-c8b9e31c20b1', 'iron_vein', 4400.9, 10474.2, 61),
    ('c847eda3-15f7-42f9-81bc-636a1a8c13c5', 'iron_vein', 2510.1, 5831.2, 30),
    ('c8e872cc-fcb7-4c45-a073-01da6e8b2d57', 'iron_vein', 3764.3, 5915.4, 38),
    ('c9553f07-7818-4c0f-84a4-42935573ff72', 'iron_vein', 8507.7, 3854.8, 49),
    ('c9f73a54-9e30-4fc0-a537-dc5812f16914', 'iron_vein', 10378.9, 5760.8, 75),
    ('ce601588-fdd7-43b1-af62-816ece2828b0', 'iron_vein', 7924.7, 9725.5, 52),
    ('cfa860f0-a8f8-4c3b-a9c1-478bb7fcf050', 'iron_vein', 5101.1, 10138.4, 31),
    ('da8c2a2a-0ecf-41ef-a45d-39dc5495a3a1', 'iron_vein', 10606.7, 7529.9, 77),
    ('df8d81d1-16a7-447d-a8ba-422284fb731c', 'iron_vein', 3454.9, 6438.6, 83),
    ('e1956b90-8957-4931-a75d-39cbed60c9f5', 'iron_vein', 11034.4, 8488.6, 67),
    ('e37d80ec-073e-4fdb-b382-aa0cc5390e0f', 'iron_vein', 7109.3, 2517.8, 28),
    ('e3df6def-d82e-4202-bad9-4383e49c776b', 'iron_vein', 11054.9, 8482.5, 68),
    ('ed8cabf5-1bab-43c0-990e-d1f6a229bc2a', 'iron_vein', 11035.2, 8520.8, 57),
    ('f399d051-5ab4-451a-94b7-cc50da5d02e3', 'iron_vein', 8071.7, 2702.9, 47),
    ('f63f9b51-0cfd-48cf-8410-8f25628659c0', 'iron_vein', 10355.1, 5738.0, 67),
    ('f66fed41-3f7a-42b9-a092-5b08c4c41115', 'iron_vein', 6176.7, 3744.6, 41),
    ('f691e18a-ce72-4648-8c18-982116175955', 'iron_vein', 2806.6, 5599.2, 39),
    ('f86503c4-fdf5-4af1-998e-280da1657877', 'iron_vein', 11073.7, 8487.1, 27),
    ('f977355d-086e-420c-8037-35fd5b2407bf', 'iron_vein', 5970.2, 3221.9, 61),
    ('fde8c9bb-b55e-4860-9dd5-c91e9cd76ec5', 'iron_vein', 8050.5, 2676.3, 63),
    ('fe0deb00-c876-4a94-b4a5-22defafd6564', 'iron_vein', 3801.8, 5915.0, 62)
ON CONFLICT (id) DO NOTHING;

-- The first recipe that needs a station (P6.3, #45).
--
-- The forge kit had no recipe at all, and no recipe needed a station, so
-- craft_at_station could never run. A forge is built at a workbench: the
-- workbench is where hand-made rope becomes something bigger.
INSERT INTO recipes (id, name, description, category, inputs, outputs, base_duration_ticks, quality_weights, crafting_requirements, required_station_type)
VALUES
    ('craft_forge', 'Forge Kit', 'Build a forge kit at a workbench, from rope and fiber.', 'station_crafting',
     '[{"item_type": "rope", "quantity": 4, "quality_weight": 0.7}, {"item_type": "fiber", "quantity": 6, "quality_weight": 0.3}]',
     '[{"item_type": "forge_kit", "quantity_min": 1, "quantity_max": 1}]',
     120,
     '{"variance_min": -3, "variance_max": 3}',
     '{}',
     'workbench')
ON CONFLICT (id) DO NOTHING;

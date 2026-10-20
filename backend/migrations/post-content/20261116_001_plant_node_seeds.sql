-- The nodes E2.3 (#162) and the wilds (#199, #225) staged in
-- resource_node_seeds, planted (#207).
--
-- Those migrations ran before the content sync had made the nodes' resource
-- types, so they couldn't insert into resource_nodes (resource_type is a
-- foreign key). Startup's `node_spawn::plant_seeds` moved each row in once its
-- type existed. This set runs after the sync, so it can do that once, here,
-- and the staging table goes. A database that already planted them has only
-- the rows it couldn't, and a node already there by id stays as it is.
--
-- With no join on resource_types, a seed whose type the content doesn't have
-- fails the foreign key and stops startup, rather than waiting for ever.
INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality)
SELECT id, resource_type, position_x, position_y, base_quality
FROM resource_node_seeds
ON CONFLICT (id) DO NOTHING;

DROP TABLE resource_node_seeds;

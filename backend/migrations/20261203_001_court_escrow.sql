-- The Contract Court's escrow box (commons plan D5, S0.2): a commission's
-- delivered units wait here, as a stack with no slot and no owner, until
-- they're paid for and go on to the poster, or go back. Like the market's
-- escrow and the bazaar's stock, nobody can move them but the court, and the
-- ledger counts them among every unit in the world. Moving units in and out
-- of it counts no unit flow.
INSERT INTO container_types (id, name, slot_count, layout_columns) VALUES
    ('court_escrow', 'Court Escrow', 0, 1);

-- One box, shared by every commission
CREATE UNIQUE INDEX idx_containers_one_court_escrow ON containers (container_type)
    WHERE container_type = 'court_escrow';
INSERT INTO containers (id, container_type, owner_id) VALUES (gen_random_uuid(), 'court_escrow', NULL);

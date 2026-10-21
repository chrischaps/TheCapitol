-- The bazaar becomes a quarter (E3.12, economy plan D6). Its 13 stalls in
-- one building across the avenue from the hall gave way to some forty along
-- the trade ring, a lane and an alley (`models::bazaar`). The ids
-- `bazaar_stall_1` and up are reused, but each now names another place, so
-- every open rental closes here rather than wake up somewhere else.
--
-- The days not used come back to the merchant (whole Strands, rounded down,
-- at the rent of 20 a day, `rules::bazaar::RENT_PER_DAY`), as `bazaar_refund`
-- in the audit. Then the rent is marked run out, and the bazaar's sweep
-- (every 10 s, `engine::bazaar::sweep`) closes each one as it closes any
-- lapsed stall: its goods to the merchant's market box, the rest waiting
-- there as `returning`, and a `stall_lapsed` notice. No goods are lost.

WITH refunds AS (
    SELECT player_id, stall_id,
           FLOOR(EXTRACT(EPOCH FROM (paid_until - NOW())) * 20 / 86400)::BIGINT AS amount
    FROM bazaar_rentals
    WHERE paid_until > NOW()
), paid AS (
    UPDATE players p
    SET strand_balance = p.strand_balance + r.amount
    FROM refunds r
    WHERE p.id = r.player_id AND r.amount > 0
    RETURNING p.id, r.stall_id, r.amount, p.strand_balance - r.amount AS balance_before, p.strand_balance AS balance_after
)
INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, description)
SELECT id, amount, balance_before, balance_after, 'bazaar_refund',
       'Rent refunded: ' || stall_id || ' closed when the bazaar became a quarter (E3.12)'
FROM paid;

UPDATE bazaar_rentals SET paid_until = NOW() WHERE paid_until > NOW();

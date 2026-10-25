-- One deed price for the three home zones (#292, Chris 2026-10-24): rural,
-- suburban and urban lots are of equal value on balance, each with its pros
-- and cons (a farm's room against the walk to the market, a yard's
-- nearness against its four stations), so they cost the same. 500, the
-- middle of the old 250 / 500 / 1000, keeps the deed sink about where it
-- was. Tax follows the price: 10 a week on any of them at 2%.
--
-- Owned plots keep the assessed value they were bought or granted at, so a
-- refund on foreclosure is still what was paid. The guild district's 5000
-- is a hall, not a home, and stays.
UPDATE zones SET deed_cost = 500 WHERE id IN ('rural', 'suburban', 'urban');

-- A deed_price knob is a multiple of its zone's deed_cost, so one an admin
-- set against the old prices (a trial of 250 everywhere is 0.5 suburban
-- and 0.25 urban) would mean 250 and 125 now. Equal prices are the
-- default, so they go back to it.
DELETE FROM tunables WHERE key IN ('deed_price.rural', 'deed_price.suburban', 'deed_price.urban');

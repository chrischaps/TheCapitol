-- E4.2 (#177): automation on stations. A crafting station's owner sets one
-- recipe to make again and again, from and into the station's own box (v1),
-- and starts or stops it. E4.3's tick does the making.
--
-- The column holds `capitol_protocol::rest::StationAutomation` as JSON:
--   {"recipe_id": "tool_handle", "mode": {"count": 20} | "forever",
--    "state": "stopped" | "running" | "paused", "reason"?: {...}, "done": 3}
-- NULL until the station is first configured. It's kept when the station is
-- stopped, so the panel can show what it last made.
ALTER TABLE stations ADD COLUMN automation JSONB;

-- E4.3 finds the stations to run without reading every row
CREATE INDEX idx_stations_automation_on ON stations ((automation->>'state'))
    WHERE automation IS NOT NULL AND automation->>'state' <> 'stopped';

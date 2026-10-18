-- E1.6 (#158): a tool slot on stations. A crafting station holds one tool,
-- which supplies the tool term for crafts made there and wears with each one.
--
-- An installed tool is an item in no container and owned by no one, so
-- nothing that takes items by owner (a trade offer, a craft input, the
-- exchange) can reach it while it's installed. Who put it there is kept on
-- the station as tool_owner: it's theirs again when it comes out, as a
-- guest's deposit in a box stays the guest's (#80).
ALTER TABLE stations
    ADD COLUMN installed_tool UUID REFERENCES items(id) ON DELETE SET NULL,
    ADD COLUMN tool_owner UUID REFERENCES players(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX idx_stations_installed_tool ON stations(installed_tool)
    WHERE installed_tool IS NOT NULL;

-- Taking a station down (#88) and foreclosure (#137) give the tool back
-- first. A station deleted any other way (a forced terrain commit, #141)
-- would leave its tool loose in no container, so it goes with the station.
CREATE FUNCTION delete_detached_station_tool() RETURNS trigger AS $$
BEGIN
    IF OLD.installed_tool IS NOT NULL THEN
        DELETE FROM items WHERE id = OLD.installed_tool AND container_id IS NULL;
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stations_delete_detached_tool
    AFTER DELETE ON stations
    FOR EACH ROW EXECUTE FUNCTION delete_detached_station_tool();

use std::collections::HashMap;
use uuid::Uuid;

use crate::models::{PlayerState, StationState, StationType};

/// Maximum distance from player to station for interaction (world units)
pub use capitol_protocol::rules::STATION_INTERACTION_RANGE;

/// Check if a player is within interaction range of a station
pub fn is_player_near_station(player: &PlayerState, station: &StationState, range: f32) -> bool {
    let dx = player.x - station.x;
    let dy = player.y - station.y;
    let distance = (dx * dx + dy * dy).sqrt();
    distance <= range as f64
}

/// Check if a player is within interaction range of a station (using station type's range)
pub fn is_player_near_station_type(
    player: &PlayerState,
    station: &StationState,
    station_types: &HashMap<String, StationType>,
) -> bool {
    let range = station_types
        .get(&station.station_type)
        .map(|st| st.interaction_range)
        .unwrap_or(STATION_INTERACTION_RANGE as f32);
    is_player_near_station(player, station, range)
}

/// Get all stations within range of a position
pub fn get_nearby_stations<'a>(
    stations: &'a HashMap<Uuid, StationState>,
    x: f64,
    y: f64,
    range: f64,
) -> Vec<&'a StationState> {
    stations
        .values()
        .filter(|station| {
            let dx = x - station.x;
            let dy = y - station.y;
            let distance = (dx * dx + dy * dy).sqrt();
            distance <= range
        })
        .collect()
}

/// Find a station of a specific type near the player
pub fn find_nearby_station_of_type<'a>(
    player: &PlayerState,
    station_type: &str,
    stations: &'a HashMap<Uuid, StationState>,
    station_types: &HashMap<String, StationType>,
) -> Option<&'a StationState> {
    let range = station_types
        .get(station_type)
        .map(|st| st.interaction_range)
        .unwrap_or(STATION_INTERACTION_RANGE as f32) as f64;

    get_nearby_stations(stations, player.x, player.y, range)
        .into_iter()
        .find(|station| station.station_type == station_type)
}

/// Check if player can access a station (owner check for now)
pub fn can_player_access_station(player: &PlayerState, station: &StationState) -> bool {
    // For now, only owner can access their stations
    station.owner_id == player.id
}

/// Check minimum distance from other stations for placement
pub fn is_valid_placement(
    x: f64,
    y: f64,
    stations: &HashMap<Uuid, StationState>,
    min_distance: f64,
) -> bool {
    for station in stations.values() {
        let dx = x - station.x;
        let dy = y - station.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance < min_distance {
            return false;
        }
    }
    true
}

/// Get station kit item type for a station type
pub fn get_station_kit_type(station_type: &str) -> Option<&'static str> {
    capitol_protocol::rules::station_kit_item(station_type)
}

/// Get station type from kit item type
pub fn get_station_type_from_kit(item_type: &str) -> Option<&'static str> {
    capitol_protocol::rules::station_type_for_kit(item_type)
}

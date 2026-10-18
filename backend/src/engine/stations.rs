use std::collections::HashMap;

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

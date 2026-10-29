//! What living in each home zone is like (the road in's zone card, #444,
//! plan D9): `GET /bureaucracy/zone-facts`.
//!
//! The card tells the truth. Its stations are what a lot **fits** at
//! `STATION_MIN_SPACING`: since the building came with the deed (Plots v2
//! §5.6, V3.1, #473), its room indoors and in the yard, packed by
//! `station_room::room` as the sync packs it into `plots.indoor_room` and
//! `yard_room`. That's the lot's capacity too, before floors. Its walk is
//! the straight line from a lot to the Exchange hall at `PLAYER_SPEED`.
//!
//! Packing every lot takes a while (the default world's 3000, a few seconds
//! in debug), so the room and the walk are worked out once for each world
//! ([`rooms`]), on a blocking thread: warmed at startup, and again on the
//! first ask after a terrain commit, keyed by the terrain's version.
//! The price, the tax and the free lots are read live on every ask.
//!
//! The guild district's lots are packed with them, for a leader choosing a
//! hall (#581): no building stands on one, so it's all yard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::rest::{FreeLot, ZoneFacts, ZoneGrowth};
use capitol_protocol::rules::{Zone, PLAYER_SPEED};
use capitol_protocol::terrain::{LandmarkKind, TerrainPlot};
use capitol_protocol::tunables::Tunables;
use sqlx::PgPool;

use crate::models::station_room;
use crate::models::ResourceType;
use crate::state::AppState;

/// The zones a player can make a home in, from the city out (D6).
pub const HOME_ZONES: [Zone; 3] = [Zone::Urban, Zone::Suburban, Zone::Rural];

/// The zones whose free lots are listed: the homes, and the guild district
/// for a guild's hall (#581).
pub fn lists(zone: Zone) -> bool {
    HOME_ZONES.contains(&zone) || zone == Zone::GuildDistrict
}

/// What living in each zone is like, in one line: the plots plan's §1.
pub fn style(zone: Zone) -> &'static str {
    match zone {
        Zone::Urban => "A workshop yard a step from the market.",
        Zone::Suburban => "A house and a garden on a quiet street, with room for a trade.",
        Zone::Rural => "A farm on a lane, with room to spread out.",
        _ => "",
    }
}

/// One lot, as the card reads it.
#[derive(Debug, Clone)]
struct Lot {
    /// Its id in `/terrain`
    id: String,
    zone: Zone,
    corners: [(f64, f64); 4],
    area: f64,
    /// The stations it fits: its room indoors and in the yard
    fits: i32,
    /// Its capacity, the same before floors
    capacity: i32,
    /// Its middle's straight walk to the hall, in seconds
    walk: f64,
    /// Its address: "7 Larch Close" (#483)
    address: Option<String>,
}

/// Every home lot of one world, packed.
pub struct Rooms {
    /// The terrain version they were packed for
    version: String,
    lots: Vec<Lot>,
}

/// The last world packed. One at a time: a commit replaces it.
static ROOMS: Mutex<Option<Arc<Rooms>>> = Mutex::new(None);

/// The home lots of the world being played, packed: from the cache when the
/// terrain hasn't changed since, otherwise packed now, off the async threads.
pub async fn rooms(state: &AppState) -> Result<Arc<Rooms>, sqlx::Error> {
    let (version, plots, hall, addresses) = {
        let game = state.game.read().await;
        let version = game.terrain_version.clone();
        if let Some(rooms) = ROOMS.lock().unwrap().as_ref().filter(|r| r.version == version) {
            return Ok(rooms.clone());
        }
        let terrain = &game.terrain;
        let plots: Vec<TerrainPlot> = terrain
            .terrain_plots
            .iter()
            .filter(|p| Zone::from_str(&p.zone).is_some_and(lists))
            .cloned()
            .collect();
        let hall = terrain
            .landmarks
            .iter()
            .find(|l| l.kind == LandmarkKind::ExchangeHall)
            .map_or((terrain.center_x, terrain.center_y), |l| l.door);
        (version, plots, hall, game.addresses.clone())
    };
    let rooms = tokio::task::spawn_blocking(move || Arc::new(pack(version, &plots, hall, &addresses)))
        .await
        .expect("packing the lots panicked");
    *ROOMS.lock().unwrap() = Some(rooms.clone());
    Ok(rooms)
}

fn pack(version: String, plots: &[TerrainPlot], hall: (f64, f64), addresses: &HashMap<String, String>) -> Rooms {
    let lots = plots
        .iter()
        .filter_map(|p| {
            let zone = Zone::from_str(&p.zone)?;
            let fits = match station_room::room(&p.corners, zone) {
                Some(room) => room.capacity(0),
                // A hall's lot has no building (#581)
                None if zone == Zone::GuildDistrict => station_room::open_ground(&p.corners),
                None => return None,
            } as i32;
            let capacity = fits;
            let (mx, my) = middle(&p.corners);
            let walk = (mx - hall.0).stable_hypot(my - hall.1) / PLAYER_SPEED;
            let address = addresses.get(&p.id).cloned();
            Some(Lot { id: p.id.clone(), zone, corners: p.corners, area: p.area, fits, capacity, walk, address })
        })
        .collect();
    Rooms { version, lots }
}

fn middle(c: &[(f64, f64); 4]) -> (f64, f64) {
    (c.iter().map(|p| p.0).sum::<f64>() / 4.0, c.iter().map(|p| p.1).sum::<f64>() / 4.0)
}

/// The middle value, the upper of the two for an even count.
fn median<T: Copy + PartialOrd>(mut xs: Vec<T>) -> Option<T> {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    xs.get(xs.len() / 2).copied()
}

/// What doesn't change between asks: one zone's room, walk and typical lot.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub lots: i64,
    pub fits_median: i32,
    pub fits_least: i32,
    pub capacity: i32,
    pub walk_nearest: f64,
    pub walk_median: f64,
    pub typical: [(f64, f64); 4],
    pub typical_area: f64,
}

impl Rooms {
    /// `zone`'s room, or `None` when the world has no lots there.
    pub fn room(&self, zone: Zone) -> Option<Room> {
        let lots: Vec<&Lot> = self.lots.iter().filter(|l| l.zone == zone).collect();
        let fits_median = median(lots.iter().map(|l| l.fits).collect())?;
        let area_median = median(lots.iter().map(|l| l.area).collect())?;
        // The typical lot fits the median, and is of about the median size.
        let typical = lots
            .iter()
            .filter(|l| l.fits == fits_median)
            .min_by(|a, b| (a.area - area_median).abs().total_cmp(&(b.area - area_median).abs()))?;
        let (mx, my) = middle(&typical.corners);
        Some(Room {
            lots: lots.len() as i64,
            fits_median,
            fits_least: lots.iter().map(|l| l.fits).min()?,
            capacity: typical.capacity,
            walk_nearest: lots.iter().map(|l| l.walk).fold(f64::MAX, f64::min),
            walk_median: median(lots.iter().map(|l| l.walk).collect())?,
            typical: typical.corners.map(|(x, y)| (x - mx, y - my)),
            typical_area: typical.area,
        })
    }
}

/// What grows in `zone`, densest first, from the spawn tables.
pub fn grows(zone: Zone, resource_types: &HashMap<String, ResourceType>) -> Vec<ZoneGrowth> {
    let mut grows: Vec<ZoneGrowth> = resource_types
        .values()
        .filter_map(|r| {
            let spawn = r.spawns.0.iter().find(|s| s.zone == zone.as_str() && s.density > 0.0)?;
            Some(ZoneGrowth { resource_id: r.id.clone(), name: r.name.clone(), density: spawn.density })
        })
        .collect();
    grows.sort_by(|a, b| b.density.total_cmp(&a.density).then_with(|| a.resource_id.cmp(&b.resource_id)));
    grows
}

/// Claimed lots by zone, now.
async fn claimed(db: &PgPool) -> Result<HashMap<String, i64>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT zone_id, COUNT(*) FROM plots
         WHERE owner_id IS NOT NULL AND terrain_id IS NOT NULL
         GROUP BY zone_id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Each home zone's facts: the room from [`rooms`], the rest live.
pub async fn facts(state: &AppState) -> Result<Vec<ZoneFacts>, sqlx::Error> {
    let rooms = rooms(state).await?;
    let claimed = claimed(&state.db).await?;
    let base: HashMap<String, i32> = sqlx::query_as::<_, (String, i32)>(
        "SELECT id, deed_cost FROM zones WHERE deed_cost IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let tunables = state.tunables.get();
    let game = state.game.read().await;
    Ok(HOME_ZONES
        .iter()
        .filter_map(|&zone| {
            let room = rooms.room(zone)?;
            let deed_cost = tunables.deed_cost(zone.as_str(), *base.get(zone.as_str())?);
            Some(assemble(zone, &room, deed_cost, &tunables, claimed.get(zone.as_str()).copied().unwrap_or(0), grows(zone, &game.resource_types)))
        })
        .collect())
}

/// Every lot free in `zone` now, nearest the hall first, with the stations
/// each fits and its walk (the Deeds Office, #448; a guild's hall, #581).
/// The room and the walk come from [`rooms`]; which lots are free is read
/// live.
pub async fn free_lots(state: &AppState, zone: Zone) -> Result<Vec<FreeLot>, sqlx::Error> {
    let rooms = rooms(state).await?;
    let free: HashMap<String, Free> = sqlx::query_as::<_, (String, uuid::Uuid, Option<i32>)>(
        "SELECT p.terrain_id, p.id, psc.station_capacity FROM plots p
         LEFT JOIN plot_size_categories psc ON psc.id = p.size_category
         WHERE p.owner_id IS NULL AND p.terrain_id IS NOT NULL AND p.zone_id = $1",
    )
    .bind(zone.as_str())
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(terrain_id, plot_id, size_capacity)| (terrain_id, Free { plot_id, size_capacity }))
    .collect();
    Ok(listed(&rooms, zone, &free))
}

/// A free lot's plot, as the database has it.
#[derive(Debug, Clone, Copy)]
struct Free {
    plot_id: uuid::Uuid,
    /// Its size category's capacity, which is what a lot with no building
    /// (a guild hall's) holds (`plot::STATION_CAPACITY`)
    size_capacity: Option<i32>,
}

/// `zone`'s lots of `rooms` that are in `free` (by terrain id), nearest the
/// hall first
fn listed(rooms: &Rooms, zone: Zone, free: &HashMap<String, Free>) -> Vec<FreeLot> {
    let housed = capitol_protocol::rules::building::shape(zone).is_some();
    let mut lots: Vec<FreeLot> = rooms
        .lots
        .iter()
        .filter(|l| l.zone == zone)
        .filter_map(|l| {
            let free = free.get(&l.id)?;
            let (x, y) = middle(&l.corners);
            let capacity = match free.size_capacity {
                Some(capacity) if !housed => capacity,
                _ => l.capacity,
            };
            Some(FreeLot {
                plot_id: free.plot_id,
                terrain_id: l.id.clone(),
                x,
                y,
                area: l.area,
                fits: l.fits.min(capacity),
                capacity,
                walk_secs: l.walk,
                address: l.address.clone(),
            })
        })
        .collect();
    lots.sort_by(|a, b| a.walk_secs.total_cmp(&b.walk_secs).then_with(|| a.terrain_id.cmp(&b.terrain_id)));
    lots
}

fn assemble(zone: Zone, room: &Room, deed_cost: i32, tunables: &Tunables, claimed: i64, grows: Vec<ZoneGrowth>) -> ZoneFacts {
    ZoneFacts {
        zone_id: zone.as_str().to_owned(),
        zone_name: zone.display_name().to_owned(),
        style: style(zone).to_owned(),
        deed_cost,
        tax_per_week: crate::engine::property_tax::weekly_tax(deed_cost, tunables.property_tax_rate),
        stations_median: room.fits_median,
        stations_least: room.fits_least,
        station_capacity: room.capacity,
        lots_free: (room.lots - claimed).max(0),
        lots_total: room.lots,
        walk_nearest_secs: room.walk_nearest,
        walk_median_secs: room.walk_median,
        grows,
        typical_lot: room.typical,
        typical_area: room.typical_area,
    }
}

/// The database tests need `DATABASE_URL`, the local Redis and the default
/// terrain, so they're ignored with the rest and run in `ci.sh db`:
/// `cargo test -p capitol-backend zone_facts:: -- --include-ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{player, plot};
    use crate::state::for_tests;

    #[test]
    fn grows_lists_a_zones_resources_densest_first() {
        let types: HashMap<String, ResourceType> = ["grass", "fallen_branches", "rock_outcrop", "tree", "clay_bank"]
            .into_iter()
            .map(|id| (id.to_owned(), crate::content::resource_type(id)))
            .collect();
        let rural: Vec<String> = grows(Zone::Rural, &types).into_iter().map(|g| g.resource_id).collect();
        assert_eq!(rural, ["tree", "fallen_branches", "rock_outcrop", "grass"]);
        let urban: Vec<String> = grows(Zone::Urban, &types).into_iter().map(|g| g.resource_id).collect();
        assert_eq!(urban, ["clay_bank"]);
        assert_eq!(grows(Zone::Rural, &types)[0].name, "Tree");
    }

    #[test]
    fn median_takes_the_middle() {
        assert_eq!(median(vec![3, 1, 2]), Some(2));
        assert_eq!(median(vec![4, 1, 2, 3]), Some(3));
        assert_eq!(median::<i32>(vec![]), None);
    }

    /// The facts match the default world (Plots v2, #459): an urban lot fits
    /// 4, a suburban one 6 or more, a farm 12 or more, each its building's
    /// room and its capacity (#473); the walk to the hall grows outward; a claim takes a
    /// lot off the free count.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_facts_match_the_default_world(db: PgPool) {
        let state = for_tests(db.clone()).await;
        let owner = player(&db, "Homer", 0).await;
        plot(&db, Some(owner), 1000).await;
        let facts = facts(&state).await.unwrap();
        let ids: Vec<&str> = facts.iter().map(|f| f.zone_id.as_str()).collect();
        assert_eq!(ids, ["urban", "suburban", "rural"]);
        let [urban, suburban, rural] = [&facts[0], &facts[1], &facts[2]];
        for f in &facts {
            println!(
                "{}: deed {} tax {} fits {} (least {}) of {} | {} of {} free | walk {:.0}..{:.0} s | {:.0} sq | {:?}",
                f.zone_id,
                f.deed_cost,
                f.tax_per_week,
                f.stations_median,
                f.stations_least,
                f.station_capacity,
                f.lots_free,
                f.lots_total,
                f.walk_nearest_secs,
                f.walk_median_secs,
                f.typical_area,
                f.grows.iter().map(|g| &g.resource_id).collect::<Vec<_>>()
            );
            assert!(!f.style.is_empty());
            assert_eq!(f.tax_per_week, (f.deed_cost as f64 * capitol_protocol::tunables::PROPERTY_TAX_RATE).round() as i64);
            assert!(f.stations_least <= f.stations_median && f.stations_median <= f.station_capacity);
            assert!(f.walk_nearest_secs <= f.walk_median_secs);
            assert!(f.lots_total > 800, "about 1000 lots a zone: {}", f.lots_total);
            assert!((f.typical_area - TerrainPlot::calculate_area(&f.typical_lot)).abs() < 1.0);
        }
        assert_eq!(urban.stations_median, 4, "an urban lot fits 4");
        assert_eq!(urban.station_capacity, 4, "and its capacity, its building's room, says so too (#473)");
        assert!(suburban.stations_median >= 6, "a suburban lot fits a workshop: {}", suburban.stations_median);
        assert!(rural.stations_median >= 12, "a farm fits 12: {}", rural.stations_median);
        assert!(urban.walk_nearest_secs < suburban.walk_nearest_secs && suburban.walk_nearest_secs < rural.walk_nearest_secs);
        assert_eq!(urban.lots_free, urban.lots_total - 1, "the claim is off the free count");
        assert_eq!(suburban.lots_free, suburban.lots_total);
    }

    /// The Deeds Office's list (#448): only the zone's free lots, each a
    /// plot to claim, nearest the hall first, with the card's room and walk
    #[test]
    fn the_free_lots_are_the_zones_unclaimed_nearest_first() {
        let lot = |id: &str, zone, walk| Lot {
            id: id.to_owned(),
            zone,
            corners: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
            area: 100.0,
            fits: 4,
            capacity: 16,
            walk,
            address: Some(format!("3 {id} Lane")),
        };
        let rooms = Rooms {
            version: "v".into(),
            lots: vec![
                lot("far", Zone::Rural, 90.0),
                lot("near", Zone::Rural, 70.0),
                lot("taken", Zone::Rural, 60.0),
                lot("yard", Zone::Urban, 20.0),
            ],
        };
        let free: HashMap<String, Free> = ["far", "near", "yard"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id.to_owned(), Free { plot_id: uuid::Uuid::from_u128(i as u128 + 1), size_capacity: Some(9) }))
            .collect();
        let listed = listed(&rooms, Zone::Rural, &free);
        let ids: Vec<&str> = listed.iter().map(|l| l.terrain_id.as_str()).collect();
        assert_eq!(ids, ["near", "far"]);
        assert_eq!(listed[0].plot_id, uuid::Uuid::from_u128(2));
        assert_eq!((listed[0].x, listed[0].y, listed[0].fits, listed[0].walk_secs), (5.0, 5.0, 4, 70.0));
        assert_eq!(listed[0].address.as_deref(), Some("3 near Lane"));
        assert_eq!(listed[0].capacity, 16, "a home lot holds its building's room, not its size category's");
    }

    /// A hall's lot (#581) has no building, so it holds its size category's
    /// capacity, and fits no more than that
    #[test]
    fn a_halls_lot_holds_its_size_categorys_capacity() {
        let corners = [(0.0, 0.0), (300.0, 0.0), (300.0, 200.0), (0.0, 200.0)];
        let lot = |id: &str, fits| Lot {
            id: id.to_owned(),
            zone: Zone::GuildDistrict,
            corners,
            area: 60_000.0,
            fits,
            capacity: fits,
            walk: 30.0,
            address: None,
        };
        assert!(station_room::open_ground(&corners) > 20, "a hall's lot is all yard");
        let rooms = Rooms { version: "v".into(), lots: vec![lot("roomy", 40), lot("snug", 6)] };
        let free: HashMap<String, Free> = [("roomy", 1), ("snug", 2)]
            .into_iter()
            .map(|(id, n)| (id.to_owned(), Free { plot_id: uuid::Uuid::from_u128(n), size_capacity: Some(25) }))
            .collect();
        let listed = listed(&rooms, Zone::GuildDistrict, &free);
        let held: Vec<(i32, i32)> = listed.iter().map(|l| (l.fits, l.capacity)).collect();
        assert_eq!(held, [(25, 25), (6, 25)]);
    }

    /// On the default world every free lot is listed, and a claimed one isn't
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_offices_list_is_the_default_worlds_free_lots(db: PgPool) {
        let state = for_tests(db.clone()).await;
        // The plot rows, as startup's sync makes them
        let lots = state.game.read().await.terrain.terrain_plots.clone();
        crate::engine::plot_sync::sync_plots(&db, &lots, crate::engine::plot_sync::OnOwned::Keep).await.unwrap();
        let facts = facts(&state).await.unwrap();
        let rural = facts.iter().find(|f| f.zone_id == "rural").unwrap();
        let before = free_lots(&state, Zone::Rural).await.unwrap();
        assert_eq!(before.len() as i64, rural.lots_free);
        let owner = player(&db, "Homer", 0).await;
        sqlx::query("UPDATE plots SET owner_id = $1 WHERE id = $2").bind(owner).bind(before[0].plot_id).execute(&db).await.unwrap();
        let after = free_lots(&state, Zone::Rural).await.unwrap();
        assert_eq!(after.len(), before.len() - 1);
        assert!(after.iter().all(|l| l.plot_id != before[0].plot_id));
        assert!(after.iter().all(|l| l.fits <= l.capacity));
        // The guild district's are listed for a hall (#581), with no card of
        // their own: every one of its lots, holding its size category's
        let halls = free_lots(&state, Zone::GuildDistrict).await.unwrap();
        let district = lots.iter().filter(|l| l.zone == Zone::GuildDistrict.as_str()).count();
        println!("{} free halls, the nearest {:.0} s from the hall, fitting {:?}", halls.len(), halls[0].walk_secs, halls.iter().map(|l| l.fits).collect::<Vec<_>>());
        assert_eq!(halls.len(), district);
        assert!(halls.iter().all(|l| l.fits > 0 && l.fits <= l.capacity), "{halls:?}");
        assert!(facts.iter().all(|f| f.zone_id != "guild_district"), "the zone card is the homes'");
    }
}

//! The wilds' relief for the console's map (#415): `GET /admin/relief?size=`.
//!
//! The console is plain JS with no build step (D1), so it can't run
//! `land::height_at`. The server samples the carved ground once a terrain
//! and size, and sends it as a small PNG of numbers, not colours, so the
//! page tints it to its own theme:
//!
//! - **red** is the height, 1 to 255 over `X-Relief-Peak` units (the highest
//!   ground it drew), and 0 off the wilds: inside the rural edge, where the
//!   zones' washes show through, and past the edge of the world;
//! - **green** is a hillshade lit from the north-west, Lambert's cosine to 255.
//!   Flat ground is [`FLAT_SHADE`];
//! - **blue** is 0.
//!
//! Made in a blocking task (a 1024 takes about 0.2 s in release, and is
//! about 470 KB) and
//! kept by terrain version, so a regenerate is seen at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::{Extension, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use capitol_protocol::land::{carve, LandParams, Relief, Water};
use capitol_protocol::terrain::{Lake, WaterCourse};
use capitol_protocol::rules::Zone;
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::TerrainConfig;
use crate::state::AppState;

/// The size a request gets without `size`.
pub const DEFAULT_SIZE: u32 = 1024;
/// Sizes are rounded to this, so the cache holds a few.
const SIZE_STEP: u32 = 256;
const MIN_SIZE: u32 = 256;
const MAX_SIZE: u32 = 2048;
/// The light: from the north-west (up and left on the map), well up.
const LIGHT: (f64, f64, f64) = (-1.0, -1.0, 1.6);
/// Green on level ground, so the page can tell lit from shaded. It's sent
/// as `X-Relief-Flat`.
pub const FLAT_SHADE: u8 = 191;

#[derive(Debug, Deserialize)]
pub struct ReliefQuery {
    pub size: Option<u32>,
}

/// A drawn relief: the PNG and the height its red 255 stands for.
#[derive(Clone)]
pub struct Drawn {
    pub png: Arc<Vec<u8>>,
    pub peak: f64,
}

/// By size, for one terrain version. A new version empties it.
#[derive(Default)]
struct Cache {
    version: String,
    drawn: HashMap<u32, Drawn>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// A size the cache keys on: clamped, and rounded to [`SIZE_STEP`].
pub fn snap(size: Option<u32>) -> u32 {
    let size = size.unwrap_or(DEFAULT_SIZE).clamp(MIN_SIZE, MAX_SIZE);
    ((size + SIZE_STEP / 2) / SIZE_STEP * SIZE_STEP).clamp(MIN_SIZE, MAX_SIZE)
}

/// The wilds' relief as a PNG of height and shade (see the module's note),
/// `size` pixels square over the whole world. `ETag` is the terrain's
/// version and the size, `X-Relief-Peak` the height red 255 stands for, and
/// `X-Relief-Flat` the green of level ground.
pub async fn get_relief(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<ReliefQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    let size = snap(query.size);
    let (version, terrain) = {
        let game = state.game.read().await;
        let version = game.terrain_version.clone();
        let hit = {
            let cache = cache().lock().expect("the relief cache");
            (cache.version == version).then(|| cache.drawn.get(&size).cloned()).flatten()
        };
        match hit {
            Some(drawn) => return Ok(answer(&version, size, drawn, &headers)),
            None => (version, Ground::of(&game.terrain)),
        }
    };

    let drawn = tokio::task::spawn_blocking(move || draw(&terrain, size as usize))
        .await
        .map_err(|e| AppError::Internal(format!("drawing the relief: {e}")))?;
    {
        let mut cache = cache().lock().expect("the relief cache");
        if cache.version != version {
            *cache = Cache { version: version.clone(), drawn: HashMap::new() };
        }
        cache.drawn.insert(size, drawn.clone());
    }
    Ok(answer(&version, size, drawn, &headers))
}

fn answer(version: &str, size: u32, drawn: Drawn, headers: &HeaderMap) -> Response {
    let etag = format!("W/\"{version}-{size}\"");
    let peak = HeaderValue::from_str(&format!("{:.1}", drawn.peak)).expect("a number");
    let common = [
        (header::ETAG, HeaderValue::from_str(&etag).expect("a version")),
        // Stored, but always revalidated, so a regenerated terrain is seen at once
        (header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache")),
        (HeaderName::from_static("x-relief-peak"), peak),
        (HeaderName::from_static("x-relief-flat"), HeaderValue::from(FLAT_SHADE)),
    ];
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    if unchanged {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    let mut response = (common, drawn.png.as_ref().clone()).into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    response
}

/// What the relief is drawn from: the terrain's land and water, copied out
/// so the game's lock isn't held while it's drawn.
pub struct Ground {
    land: LandParams,
    world_size: f64,
    center_x: f64,
    center_y: f64,
    courses: Vec<WaterCourse>,
    lakes: Vec<Lake>,
}

impl Ground {
    pub fn of(terrain: &TerrainConfig) -> Self {
        Ground {
            land: terrain.land.clone(),
            world_size: terrain.world_size,
            center_x: terrain.center_x,
            center_y: terrain.center_y,
            courses: terrain.rivers.iter().chain(&terrain.streams).cloned().collect(),
            lakes: terrain.lakes.clone(),
        }
    }
}

/// Samples the carved ground over the wilds, `size` pixels square over the
/// whole world, and packs it (see the module's note).
pub fn draw(terrain: &Ground, size: usize) -> Drawn {
    let relief = Relief::new(&terrain.land);
    let water = Water::in_world(terrain.world_size, &terrain.courses, &terrain.lakes);
    let scale = terrain.world_size / size as f64;
    let (cx, cy) = (terrain.center_x, terrain.center_y);
    let inner = Zone::Rural.radii().1;
    let edge = terrain.world_size / 2.0;
    let radius = |px: usize, py: usize| {
        let (x, y) = ((px as f64 + 0.5) * scale, (py as f64 + 0.5) * scale);
        ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
    };

    // The ground at each pixel's centre, a pixel past the wilds each way so
    // the shade at their edges has neighbours
    let mut height = vec![0.0f64; size * size];
    for py in 0..size {
        for px in 0..size {
            let r = radius(px, py);
            if r >= inner - 2.0 * scale && r <= edge + 2.0 * scale {
                let (x, y) = ((px as f64 + 0.5) * scale, (py as f64 + 0.5) * scale);
                height[py * size + px] = carve(relief.height(x, y), x, y, &water);
            }
        }
    }
    let wild = |px: usize, py: usize| (inner..=edge).contains(&radius(px, py));
    let peak = (0..size * size)
        .filter(|&i| wild(i % size, i / size))
        .map(|i| height[i])
        .fold(1.0f64, f64::max);

    let at =|px: isize, py: isize| {
        let (px, py) = (px.clamp(0, size as isize - 1) as usize, py.clamp(0, size as isize - 1) as usize);
        height[py * size + px]
    };
    let mut rgb = vec![0u8; size * size * 3];
    for py in 0..size {
        for px in 0..size {
            if !wild(px, py) {
                continue;
            }
            let h = height[py * size + px];
            let (ix, iy) = (px as isize, py as isize);
            let gx = (at(ix + 1, iy) - at(ix - 1, iy)) / (2.0 * scale);
            let gy = (at(ix, iy + 1) - at(ix, iy - 1)) / (2.0 * scale);
            let i = (py * size + px) * 3;
            rgb[i] = 1 + (254.0 * (h / peak).clamp(0.0, 1.0)).round() as u8;
            rgb[i + 1] = shade(gx, gy);
        }
    }
    Drawn { png: Arc::new(png(size, size, &rgb)), peak }
}

/// The hillshade's green for ground whose height rises `gx` a unit east and
/// `gy` a unit south: Lambert's cosine to [`LIGHT`], to 255.
fn shade(gx: f64, gy: f64) -> u8 {
    let l = (LIGHT.0 * LIGHT.0 + LIGHT.1 * LIGHT.1 + LIGHT.2 * LIGHT.2).sqrt();
    let len = (gx * gx + gy * gy + 1.0).sqrt();
    let lambert = ((-gx * LIGHT.0 - gy * LIGHT.1 + LIGHT.2) / (l * len)).max(0.0);
    (255.0 * lambert).round() as u8
}

/// An RGB PNG, deflated.
fn png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let stride = w * 3;
    let mut raw = Vec::with_capacity(h * (stride + 1));
    for (j, row) in rgb.chunks(stride).enumerate() {
        // Filter "up" on every row: the land changes slowly down a column
        raw.push(2);
        if j == 0 {
            raw.extend_from_slice(row);
        } else {
            let above = &rgb[(j - 1) * stride..j * stride];
            raw.extend(row.iter().zip(above).map(|(&b, &a)| b.wrapping_sub(a)));
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).expect("deflate into memory");
    let z = z.finish().expect("deflate into memory");

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = !0u32;
    for &byte in kind.iter().chain(data) {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WorldGenParams;

    /// What `draw` packed, read back out of its PNG.
    fn unpack(png: &[u8], size: usize) -> Vec<u8> {
        let idat = png.windows(4).position(|w| w == b"IDAT").expect("an IDAT");
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap()) as usize;
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&png[idat + 4..idat + 4 + len]), &mut raw)
            .expect("inflate");
        let stride = size * 3;
        let mut rgb = vec![0u8; size * stride];
        for j in 0..size {
            let row = &raw[j * (stride + 1)..(j + 1) * (stride + 1)];
            assert_eq!(row[0], 2, "filtered up");
            for k in 0..stride {
                let above = if j == 0 { 0 } else { rgb[(j - 1) * stride + k] };
                rgb[j * stride + k] = row[k + 1].wrapping_add(above);
            }
        }
        rgb
    }

    #[test]
    fn level_ground_is_the_flat_shade_and_slopes_face_the_light_or_not() {
        assert_eq!(shade(0.0, 0.0), FLAT_SHADE);
        // Rising to the south-east faces the north-west's light
        assert!(shade(0.5, 0.5) > FLAT_SHADE);
        assert!(shade(-0.5, -0.5) < FLAT_SHADE);
    }

    #[test]
    fn sizes_snap_to_a_few() {
        assert_eq!(snap(None), DEFAULT_SIZE);
        assert_eq!(snap(Some(1)), MIN_SIZE);
        assert_eq!(snap(Some(1000)), 1024);
        assert_eq!(snap(Some(1300)), 1280);
        assert_eq!(snap(Some(99_999)), MAX_SIZE);
    }

    #[test]
    fn the_wilds_have_height_and_the_city_none() {
        let terrain = TerrainConfig::from_params(&WorldGenParams::default());
        let size = 256;
        let drawn = draw(&Ground::of(&terrain), size);
        assert!(drawn.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(drawn.peak > 100.0, "the ranges stand up: {}", drawn.peak);
        let rgb = unpack(&drawn.png, size);
        let red = |px: usize, py: usize| rgb[(py * size + px) * 3];
        // The centre is the city, a corner is past the world's edge
        assert_eq!(red(size / 2, size / 2), 0);
        assert_eq!(red(0, 0), 0);
        // Due east, half way across the wilds, is ground, and the peak is somewhere
        let r = (Zone::Rural.radii().1 + terrain.world_size / 2.0) / 2.0;
        let east = ((terrain.center_x + r) / terrain.world_size * size as f64) as usize;
        assert!(red(east, size / 2) >= 1);
        assert!((0..size * size).any(|i| rgb[i * 3] == 255));
        // and it's lit and shaded, not flat everywhere
        let greens: std::collections::BTreeSet<u8> =
            (0..size * size).filter(|&i| rgb[i * 3] > 0).map(|i| rgb[i * 3 + 1]).collect();
        assert!(greens.len() > 40, "{} shades", greens.len());
    }

    /// Writes the default world's relief as the route sends it, and says how
    /// long it took and how big it is:
    /// `RELIEF_PNG=relief.png cargo test --release dump_relief -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_relief() {
        let path = std::env::var("RELIEF_PNG").unwrap_or_else(|_| "relief.png".into());
        let size: usize = std::env::var("RELIEF_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_SIZE as usize);
        let terrain = TerrainConfig::from_params(&WorldGenParams::default());
        let ground = Ground::of(&terrain);
        let started = std::time::Instant::now();
        let drawn = draw(&ground, size);
        let took = started.elapsed();
        std::fs::write(&path, drawn.png.as_ref()).expect("write the relief");
        println!("relief: {path}, {size}px, {} bytes, peak {:.0}, drawn in {took:?}", drawn.png.len(), drawn.peak);
    }
}

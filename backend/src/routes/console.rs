//! The Observatory's console (D1.1): one page the backend serves at
//! `/admin/console`, with its script, its look and uPlot vendored beside it
//! under `backend/static/`, and the Flint client's two faces from
//! `flint-client/fonts/`. All of it is compiled in, as the content is, so
//! there's no build step and no Node, and the page works against whichever
//! server serves it.
//!
//! The page itself is open: it signs in through `/auth/login` and every
//! number it shows comes from the admin routes, which check the token. So
//! these routes sit outside `admin::router`'s auth layer.

use axum::{
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::state::AppState;

const PAGE: &str = include_str!("../../static/console.html");
const SCRIPT: &str = include_str!("../../static/console.js");
const STYLE: &str = include_str!("../../static/console.css");
const UPLOT: &str = include_str!("../../static/vendor/uPlot.iife.min.js");
const UPLOT_STYLE: &str = include_str!("../../static/vendor/uPlot.min.css");
const DISPLAY_FONT: &[u8] = include_bytes!("../../../flint-client/fonts/Cinzel.ttf");
const BODY_FONT: &[u8] = include_bytes!("../../../flint-client/fonts/SourceSans3-Regular.ttf");
const STRONG_FONT: &[u8] = include_bytes!("../../../flint-client/fonts/SourceSans3-Semibold.ttf");

/// The console's routes, under `/admin`. The page and its script are asked
/// again each load, so a new build shows at once; uPlot and the fonts only
/// change with a vendored update, so they're kept a day.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/console", get(|| async { serve(PAGE, "text/html; charset=utf-8", FRESH) }))
        .route("/console/console.js", get(|| async { serve(SCRIPT, "text/javascript; charset=utf-8", FRESH) }))
        .route("/console/console.css", get(|| async { serve(STYLE, "text/css; charset=utf-8", FRESH) }))
        .route("/console/uPlot.js", get(|| async { serve(UPLOT, "text/javascript; charset=utf-8", KEPT) }))
        .route("/console/uPlot.css", get(|| async { serve(UPLOT_STYLE, "text/css; charset=utf-8", KEPT) }))
        .route("/console/fonts/display.ttf", get(|| async { serve(DISPLAY_FONT, "font/ttf", KEPT) }))
        .route("/console/fonts/body.ttf", get(|| async { serve(BODY_FONT, "font/ttf", KEPT) }))
        .route("/console/fonts/strong.ttf", get(|| async { serve(STRONG_FONT, "font/ttf", KEPT) }))
}

const FRESH: &str = "no-cache";
const KEPT: &str = "public, max-age=86400";

fn serve(body: impl Into<axum::body::Body>, kind: &'static str, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(kind)),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        body.into(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file the page names is one this serves, so a rename can't leave
    /// the page asking for something that isn't there
    #[test]
    fn the_page_asks_only_for_what_is_served() {
        let served = [
            "/admin/console/console.js",
            "/admin/console/console.css",
            "/admin/console/uPlot.js",
            "/admin/console/uPlot.css",
        ];
        for url in served {
            assert!(PAGE.contains(url), "the page doesn't load {url}");
        }
        for font in ["display", "body", "strong"] {
            assert!(STYLE.contains(&format!("/admin/console/fonts/{font}.ttf")), "the style doesn't load {font}");
        }
        assert!(UPLOT.starts_with("/*! https://github.com/leeoniya/uPlot (v1.6.31) */"));
    }
}

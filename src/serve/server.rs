//! The axum router: the index page, the live-reload websocket, and the
//! per-view SVGs — all served from the in-memory [`Site`].

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use tower_http::set_header::SetResponseHeaderLayer;

use super::livereload::ws_handler;
use super::state::AppState;

pub(crate) fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_handler))
        .fallback(svg)
        .with_state(state)
        // Dev-mode: never cache, or edits won't show without a hard reload.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// Serve the current index page.
async fn index(State(state): State<Arc<AppState>>) -> Response {
    let site = state.site.read().await;
    Html(site.index_html.clone()).into_response()
}

/// Serve a rendered SVG by its file name, or 404.
async fn svg(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let name = req.uri().path().trim_start_matches('/');
    let site = state.site.read().await;
    match site.svgs.get(name) {
        Some(svg) => ([(header::CONTENT_TYPE, "image/svg+xml")], svg.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

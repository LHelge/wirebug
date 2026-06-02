//! `wirebug serve` — a live-reloading dev server.
//!
//! Renders the project into memory, serves the views over HTTP, and watches
//! the project for changes: each save re-runs the pipeline and pushes a
//! reload to connected browsers over a websocket. Nothing is written to disk.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::dsl::project;

mod build;
mod livereload;
mod server;
mod state;
mod watcher;

use build::build_site;
use state::AppState;
use watcher::ProjectWatcher;

/// Serve the project at `target` (or the project containing the current
/// directory) on `port`, rebuilding and live-reloading on every change.
pub async fn serve(target: Option<&Path>, port: u16) -> anyhow::Result<()> {
    // The project root is the directory holding `wirebug.toml`; watch it whole.
    let entry = project::discover(target)
        .map_err(|problem| anyhow::anyhow!("{problem}"))
        .context("locating the project")?;
    let root = entry
        .parent()
        .context("project root has no parent directory")?
        .to_path_buf();

    let initial = build_site(target);
    initial.log(None);
    let state = Arc::new(AppState::new(initial.site));
    let router = server::build_router(Arc::clone(&state));

    let mut watcher = ProjectWatcher::new(&root, target.map(Path::to_path_buf))
        .context("starting the file watcher")?;
    let watcher_state = Arc::clone(&state);
    let watcher_handle = tokio::spawn(async move { watcher.run(&watcher_state).await });

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("serving at http://localhost:{port}");

    let shutdown_state = Arc::clone(&state);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            // Wake the watcher and every websocket handler so axum can drain.
            shutdown_state.shutdown.notify_waiters();
        })
        .await
        .context("serving")?;

    tracing::info!("stopping file watcher");
    let _ = tokio::time::timeout(Duration::from_secs(5), watcher_handle).await;
    tracing::info!("server shut down");
    Ok(())
}

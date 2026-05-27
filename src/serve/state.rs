//! Shared state for the dev server: the current rendered site, the
//! live-reload broadcast channel, and the shutdown notifier.

use indexmap::IndexMap;
use tokio::sync::{Notify, RwLock, broadcast};

/// The in-memory rendered site served over HTTP. `index_html` is served at
/// `/`; each entry in `svgs` (keyed by its file name, e.g. `pack.svg`) is
/// served at that path. Nothing is written to disk.
pub(crate) struct Site {
    pub(crate) index_html: String,
    pub(crate) svgs: IndexMap<String, String>,
}

/// Everything the HTTP handlers, the websocket, and the watcher share.
pub(crate) struct AppState {
    pub(crate) site: RwLock<Site>,
    /// Broadcast channel for signalling browsers to reload.
    pub(crate) reload_tx: broadcast::Sender<()>,
    /// Notified on shutdown so the websocket handlers and the file watcher
    /// can break out of their loops and let axum drain.
    pub(crate) shutdown: Notify,
}

impl AppState {
    /// Wrap a freshly built site with the broadcast and shutdown channels.
    pub(crate) fn new(site: Site) -> Self {
        let (reload_tx, _) = broadcast::channel(16);
        Self {
            site: RwLock::new(site),
            reload_tx,
            shutdown: Notify::new(),
        }
    }

    /// Atomically replace the rendered site and notify connected browsers to
    /// reload. The build happens elsewhere — this is the state-mutation half.
    pub(crate) async fn swap(&self, site: Site) {
        *self.site.write().await = site;
        let _ = self.reload_tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(html: &str) -> Site {
        Site {
            index_html: html.to_string(),
            svgs: IndexMap::new(),
        }
    }

    #[tokio::test]
    async fn swap_replaces_the_site() {
        let state = AppState::new(site("old"));
        state.swap(site("new")).await;
        assert_eq!(state.site.read().await.index_html, "new");
    }

    #[tokio::test]
    async fn swap_broadcasts_a_reload() {
        let state = AppState::new(site("old"));
        let mut rx = state.reload_tx.subscribe();
        state.swap(site("new")).await;
        assert!(rx.try_recv().is_ok());
    }
}

//! File watcher: re-build the site when a `.wb` file or project manifest
//! under the project root changes, debouncing bursts so one save triggers
//! one rebuild.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::build::build_site;
use super::state::AppState;

const DEBOUNCE: Duration = Duration::from_millis(200);

pub(crate) struct ProjectWatcher {
    /// Held to keep the watcher alive; dropping it stops file events.
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<()>,
    target: Option<PathBuf>,
}

impl ProjectWatcher {
    /// Watch `root` recursively for `.wb` changes. `target` is the original
    /// CLI target, replayed through `check_project` on each rebuild.
    pub(crate) fn new(root: &Path, target: Option<PathBuf>) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<()>(1);

        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) if is_project_source_change(&event) => {
                    let _ = tx.try_send(());
                }
                Ok(_) => {}
                Err(e) => tracing::error!("file watcher error: {e}"),
            },
            notify::Config::default(),
        )?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        tracing::info!(path = %root.display(), "watching project");

        Ok(Self {
            _watcher: watcher,
            rx,
            target,
        })
    }

    /// Rebuild-on-change loop until the shutdown signal fires.
    pub(crate) async fn run(&mut self, state: &Arc<AppState>) {
        // Pinned across iterations so a `notify_waiters()` that fires while
        // we're rebuilding is still observed.
        let shutdown = state.shutdown.notified();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    tracing::debug!("watcher received shutdown signal");
                    break;
                }
                result = self.rx.recv() => {
                    if result.is_none() {
                        break;
                    }
                    self.debounce().await;

                    tracing::info!("change detected, rebuilding…");
                    let start = Instant::now();
                    let build = build_site(self.target.as_deref());
                    build.log(Some(start.elapsed()));
                    state.swap(build.site).await;
                }
            }
        }
    }

    /// Coalesce a burst of events: extend the deadline on each new event,
    /// settle once `DEBOUNCE` passes with none.
    async fn debounce(&mut self) {
        let mut deadline = tokio::time::Instant::now() + DEBOUNCE;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                result = self.rx.recv() => {
                    if result.is_none() { break; }
                    deadline = tokio::time::Instant::now() + DEBOUNCE;
                }
            }
        }
    }
}

/// A content change touching at least one project source file. Filters out
/// editor metadata churn and unrelated files in the project tree.
fn is_project_source_change(event: &notify::Event) -> bool {
    let is_content = matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any)
    );
    is_content
        && event
            .paths
            .iter()
            .any(|p| p.extension().is_some_and(|ext| ext == "wb") || is_manifest(p))
}

fn is_manifest(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == crate::dsl::manifest::FILE_NAME)
}

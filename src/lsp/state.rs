//! Mutable server state carried across the message loop.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use lsp_types::Uri;

use super::complete::CompletionIndex;
use crate::dsl::project::Overlay;

#[derive(Default)]
pub(crate) struct ServerState {
    /// Open-buffer text shadowing the disk during project loads.
    pub(crate) overlay: Overlay,
    /// Open `.wb` documents: client URI → filesystem path.
    pub(crate) open: HashMap<Uri, PathBuf>,
    /// URIs we currently have diagnostics published on. The protocol has
    /// no "clear all", so a URI whose problems went away gets an explicit
    /// empty publish — without this set, fixed files keep stale squiggles.
    pub(crate) published: HashSet<Uri>,
    /// Last-good completion snapshot. Kept when a recompute yields nothing
    /// (the buffer too broken to load) — instance/port sets are stable
    /// across a keystroke, the live buffer's tokens are not ours to cache.
    pub(crate) index: CompletionIndex,
}

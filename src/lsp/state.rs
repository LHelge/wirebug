//! Mutable server state carried across the message loop.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use lsp_types::Uri;

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
}

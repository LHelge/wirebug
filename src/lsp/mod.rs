//! `wirebug lsp` — a Language Server Protocol server over stdio.
//!
//! Synchronous, like every command except `serve`: lsp-server's channel
//! loop needs no async runtime. Stdout carries the protocol, so logs must
//! go to stderr (`main.rs` configures tracing accordingly).
//!
//! v1 scope: live diagnostics (the full `check` pipeline re-run per change,
//! with open buffers shadowing the disk via [`Overlay`]) and completion.
//! Hover, goto-definition, rename, semantic tokens, and formatting are
//! deliberately later.

// `lsp_types::Uri` caches parse offsets in a `Cell`, tripping
// `mutable_key_type` on every URI-keyed map; its `Eq`/`Hash` are the
// string itself, so the lint's hazard doesn't apply.
#![allow(clippy::mutable_key_type)]

mod diagnostics;
mod line_index;
mod state;
mod uri;

use lsp_server::{Connection, Message, Notification as ServerNotification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Completion, Request as _};
use lsp_types::{
    CompletionOptions, CompletionResponse, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, PublishDiagnosticsParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};
use thiserror::Error;

use state::ServerState;

/// Transport and protocol failures that end the server. Per-request
/// problems (a malformed payload, an unknown method) are answered on the
/// wire instead and never reach this type.
#[derive(Debug, Error)]
pub enum Error {
    #[error("LSP protocol error: {0}")]
    Protocol(#[from] lsp_server::ProtocolError),
    #[error("serializing LSP payload: {0}")]
    Payload(#[from] serde_json::Error),
    #[error("LSP transport closed: {0}")]
    ChannelClosed(String),
    #[error("joining LSP io threads: {0}")]
    Io(#[from] std::io::Error),
}

/// Run the language server over stdio until the client disconnects or
/// requests shutdown.
pub fn run() -> Result<(), Error> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = serde_json::to_value(server_capabilities())?;
    connection.initialize(capabilities)?;
    tracing::info!("wirebug language server initialized");
    main_loop(&connection)?;
    // The writer thread exits only once every channel sender is gone, so
    // the connection must drop before the join — holding it here deadlocks
    // shutdown.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Full-document sync: projects are tiny and the pipeline re-runs
        // from source anyway, so incremental edits buy nothing.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
            ..CompletionOptions::default()
        }),
        ..ServerCapabilities::default()
    }
}

fn main_loop(connection: &Connection) -> Result<(), Error> {
    let mut state = ServerState::default();
    while let Ok(first) = connection.receiver.recv() {
        // Drain whatever else is already queued so a burst of keystrokes
        // (didChange per keypress under full sync) triggers one re-check,
        // not one per message.
        let mut batch = vec![first];
        while let Ok(more) = connection.receiver.try_recv() {
            batch.push(more);
        }

        let mut dirty = false;
        for message in batch {
            match message {
                Message::Request(request) => {
                    if connection.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    handle_request(connection, &state, request)?;
                }
                Message::Notification(notification) => {
                    dirty |= handle_notification(&mut state, notification);
                }
                Message::Response(_) => {}
            }
        }

        if dirty {
            publish(connection, &mut state)?;
        }
    }
    Ok(())
}

fn handle_request(
    connection: &Connection,
    _state: &ServerState,
    request: Request,
) -> Result<(), Error> {
    let response = match request.method.as_str() {
        // Items land with the completion index; an empty list keeps
        // clients from waiting on the trigger characters until then.
        Completion::METHOD => Response::new_ok(request.id, CompletionResponse::Array(Vec::new())),
        _ => Response::new_err(
            request.id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!("unsupported method `{}`", request.method),
        ),
    };
    send(connection, Message::Response(response))
}

/// Apply one notification to the server state; `true` means the document
/// set or a buffer changed and diagnostics should be recomputed.
fn handle_notification(state: &mut ServerState, notification: ServerNotification) -> bool {
    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let Ok(params) =
                serde_json::from_value::<DidOpenTextDocumentParams>(notification.params)
            else {
                return false;
            };
            let Some(path) = uri::to_path(&params.text_document.uri) else {
                return false;
            };
            state.overlay.insert(&path, params.text_document.text);
            state.open.insert(params.text_document.uri, path);
            true
        }
        DidChangeTextDocument::METHOD => {
            let Ok(params) =
                serde_json::from_value::<DidChangeTextDocumentParams>(notification.params)
            else {
                return false;
            };
            // Full sync: the last content change is the whole document.
            let path = state.open.get(&params.text_document.uri);
            let text = params.content_changes.into_iter().next_back();
            match (path, text) {
                (Some(path), Some(change)) => {
                    state.overlay.insert(path, change.text);
                    true
                }
                _ => false,
            }
        }
        DidCloseTextDocument::METHOD => {
            let Ok(params) =
                serde_json::from_value::<DidCloseTextDocumentParams>(notification.params)
            else {
                return false;
            };
            if let Some(path) = state.open.remove(&params.text_document.uri) {
                state.overlay.remove(&path);
            }
            true
        }
        // wirebug.toml changed on disk (the manifest is never overlaid).
        DidChangeWatchedFiles::METHOD => true,
        _ => false,
    }
}

/// Re-check every open project and publish per-URI diagnostics, including
/// explicit empty sets for URIs whose problems went away.
fn publish(connection: &Connection, state: &mut ServerState) -> Result<(), Error> {
    let mut by_uri = diagnostics::check_open_documents(state.open.values(), &state.overlay);
    diagnostics::clear_stale(&mut by_uri, &state.published);
    state.published = by_uri
        .iter()
        .filter(|(_, diagnostics)| !diagnostics.is_empty())
        .map(|(uri, _)| uri.clone())
        .collect();

    for (uri, diagnostics) in by_uri {
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        };
        let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
        send(connection, Message::Notification(notification))?;
    }
    Ok(())
}

fn send(connection: &Connection, message: Message) -> Result<(), Error> {
    connection
        .sender
        .send(message)
        .map_err(|err| Error::ChannelClosed(err.to_string()))
}

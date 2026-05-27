//! Websocket live-reload: the server end of the loop whose client half lives
//! in `templates/index.html` (and [`ERROR_PAGE_SCRIPT`] on the error page).

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::broadcast;

use super::state::AppState;

/// The live-reload client script, matching the one in `templates/index.html`.
/// Used by the diagnostics error page so it, too, reloads once the project
/// builds again.
pub(crate) const ERROR_PAGE_SCRIPT: &str = r#"<script>(function(){var ws=new WebSocket("ws://"+location.host+"/ws");ws.onmessage=function(){location.reload()};ws.onclose=function(){setTimeout(function(){location.reload()},1000)}})()</script>"#;

/// Handle a websocket upgrade request for live reload.
pub(crate) async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.reload_tx.subscribe();
    // Pinned across iterations: a fresh `notified()` per loop would race with
    // `notify_waiters()` firing while we were mid-send.
    let shutdown = state.shutdown.notified();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            result = rx.recv() => {
                match result {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if socket.send(Message::Text("reload".into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                if msg.is_none() {
                    break;
                }
            }
        }
    }
}

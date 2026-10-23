//! A browser front end for `QuiltSync`: the UI's static files plus one
//! WebSocket at `/ws` that carries commands and events.
//!
//! The socket speaks the protocol of the UI's `tauri::ws` module: requests
//! `{"id", "cmd", "args"}`, replies `{"id", "ok"}` or `{"id", "err"}`, and
//! every backend event as `{"event", "payload"}`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tower_http::services::ServeDir;
use tower_http::services::ServeFile;

use crate::Result;
use crate::dispatch::dispatch;
use crate::services::Services;
use crate::telemetry::prelude::*;

#[derive(Clone)]
struct AppState {
    services: Arc<Services>,
    token: Arc<str>,
}

/// A fresh random session token.
#[must_use]
pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The Trunk output folder the server hands to the browser.
fn ui_dir() -> PathBuf {
    std::env::var_os("QUILTSYNC_UI_DIR").map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/dist")),
        PathBuf::from,
    )
}

/// Bind `bind` and serve in a background task on the current runtime.
/// Returns the bound address (port 0 picks a free one).
///
/// # Errors
///
/// When the address cannot be bound.
pub async fn serve(services: Arc<Services>, bind: SocketAddr, token: String) -> Result<SocketAddr> {
    let dir = ui_dir();
    if !dir.join("index.html").exists() {
        warn!(
            "web UI: no index.html in {}; run `trunk build` in quilt-sync/ui or set QUILTSYNC_UI_DIR",
            dir.display()
        );
    }
    let statics = ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")));
    let state = AppState {
        services,
        token: token.into(),
    };
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(statics)
        .with_state(state);
    let app = if addr.ip().is_loopback() {
        let port = addr.port();
        app.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let host = request
                    .headers()
                    .get(header::HOST)
                    .and_then(|h| h.to_str().ok());
                if host_allowed(host, port) {
                    next.run(request).await
                } else {
                    StatusCode::FORBIDDEN.into_response()
                }
            },
        ))
    } else {
        warn!("web UI: bound to {addr}, so the Host header check is off");
        app
    };
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            error!("web UI server stopped: {err}");
        }
    });
    info!("web UI listening on {addr}");
    Ok(addr)
}

/// Whether `host` (the `Host` header) names this loopback server, which
/// defeats DNS rebinding: a page on another name that resolves to 127.0.0.1
/// still sends its own name.
fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|name| host.eq_ignore_ascii_case(&format!("{name}:{port}")))
}

/// An `Origin`, when sent, must name the same host and port the request was
/// sent to.
fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let origin = origin
        .to_str()
        .ok()
        .and_then(|o| o.split_once("://").map(|(_, rest)| rest));
    matches!((origin, host), (Some(o), Some(h)) if o.eq_ignore_ascii_case(h))
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    // Taken as a `Result` so the token and Origin checks answer first.
    ws: std::result::Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if query.get("t").map(String::as_str) != Some(&*state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !origin_matches_host(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match ws {
        Ok(ws) => ws.on_upgrade(move |socket| session(socket, state.services)),
        Err(rejection) => rejection.into_response(),
    }
}

#[derive(Deserialize)]
struct Request {
    id: Value,
    cmd: String,
    #[serde(default)]
    args: Value,
}

async fn session(socket: WebSocket, services: Arc<Services>) {
    // Subscribe before anything else so no event is missed after the upgrade.
    let mut events = services.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
        // Flushes the reply to a client's close, if one is queued.
        let _ = sink.close().await;
    });

    let event_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok((name, payload)) => {
                    let text = json!({ "event": name, "payload": payload }).to_string();
                    if event_tx.send(text).is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("browser missed {missed} backend events"),
                Err(RecvError::Closed) => break,
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let request: Request = match serde_json::from_str(&text) {
            Ok(request) => request,
            Err(err) => {
                warn!("web UI: unreadable request: {err}");
                // Answer the id when there is one, so the caller never hangs.
                if let Some(id) = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v.get("id").cloned())
                {
                    let reply = json!({ "id": id, "err": format!("bad request: {err}") });
                    let _ = tx.send(reply.to_string());
                }
                continue;
            }
        };
        let services = Arc::clone(&services);
        let tx = tx.clone();
        tokio::spawn(async move {
            let reply = match dispatch(&services, &request.cmd, request.args).await {
                Ok(ok) => json!({ "id": request.id, "ok": ok }),
                Err(err) => json!({ "id": request.id, "err": err }),
            };
            let _ = tx.send(reply.to_string());
        });
    }

    forwarder.abort();
    drop(tx);
    // In-flight commands hold senders; the writer ends when they finish.
    let _ = writer.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            h.insert(header::ORIGIN, origin.parse().unwrap());
        }
        h
    }

    #[test]
    fn host_must_be_loopback_on_our_port() {
        for ok in [
            "127.0.0.1:7878",
            "localhost:7878",
            "LocalHost:7878",
            "[::1]:7878",
        ] {
            assert!(host_allowed(Some(ok), 7878), "{ok}");
        }
        for bad in [
            "evil.example:7878",
            "localhost",
            "127.0.0.1",
            "localhost:9999",
            "127.0.0.2:7878",
            "localhost.evil.example:7878",
            "",
        ] {
            assert!(!host_allowed(Some(bad), 7878), "{bad}");
        }
        assert!(!host_allowed(None, 7878));
    }

    #[test]
    fn origin_must_match_host() {
        assert!(origin_matches_host(&headers(None, "127.0.0.1:7878")));
        assert!(origin_matches_host(&headers(
            Some("http://127.0.0.1:7878"),
            "127.0.0.1:7878"
        )));
        assert!(!origin_matches_host(&headers(
            Some("http://evil.example"),
            "127.0.0.1:7878"
        )));
        assert!(!origin_matches_host(&headers(
            Some("http://127.0.0.1:9999"),
            "127.0.0.1:7878"
        )));
    }
}

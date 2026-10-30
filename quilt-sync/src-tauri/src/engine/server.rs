//! The engine's HTTP server: the UI's files and one WebSocket at `/ws` that
//! carries commands and events.
//!
//! The socket speaks JSON text frames. The server sends
//! `{"hello": {"version", "desktop", "updateAvailable"}}` first; then requests
//! `{"id", "cmd", "args"}` get `{"id", "ok"}` or `{"id", "err"}`, and every
//! engine event goes to every client as `{"event", "payload"}`.
//!
//! Any website can open a socket to `127.0.0.1`, so `/ws` checks the `Host`
//! (against DNS rebinding), the `Origin`, then the token in `?t=`, before it
//! upgrades. Pages and files need no token: the page keeps it in its own
//! origin's `localStorage`, which, unlike a cookie, no other port can read.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use futures_util::SinkExt;
use futures_util::StreamExt;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::engine::desktop::Desktop;
use crate::engine::dispatch::dispatch;
use crate::engine::services::Services;
use crate::engine::telemetry::prelude::*;

/// The UI as Trunk builds it. Debug builds read the folder on each request, so
/// `trunk watch` needs no Rust rebuild; release builds embed it, and
/// `build.rs` refuses to build one without it.
#[derive(RustEmbed)]
#[folder = "../ui/dist/"]
struct Ui;

#[derive(Clone)]
struct AppState {
    services: Arc<Services>,
    desktop: Option<Arc<dyn Desktop>>,
    token: Arc<str>,
}

/// Serve on `listener` in a background task and answer its address.
///
/// # Errors
///
/// When the listener has no local address.
pub fn serve(
    listener: TcpListener,
    services: Arc<Services>,
    desktop: Option<Arc<dyn Desktop>>,
    token: String,
) -> std::io::Result<SocketAddr> {
    let addr = listener.local_addr()?;
    let app = router(&addr, services, desktop, token);
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            error!("server stopped: {err}");
        }
    });
    info!("serving the UI on http://{addr}");
    Ok(addr)
}

fn router(
    addr: &SocketAddr,
    services: Arc<Services>,
    desktop: Option<Arc<dyn Desktop>>,
    token: String,
) -> Router {
    let state = AppState {
        services,
        desktop,
        token: token.into(),
    };
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback(ui_file)
        .layer(axum::middleware::map_response(no_cache))
        .with_state(state);
    if !addr.ip().is_loopback() {
        return app;
    }
    let port = addr.port();
    app.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| async move {
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
}

/// Whether `host` (the `Host` header) names this loopback server. A page on
/// another name that resolves to `127.0.0.1` still sends its own name.
fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|name| host.eq_ignore_ascii_case(&format!("{name}:{port}")))
}

/// An `Origin`, when sent, must be the page's own: `http://` and the `Host`.
/// Browsers always send one with a socket; other clients needn't.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    match (origin.to_str(), host) {
        (Ok(origin), Some(host)) => origin.eq_ignore_ascii_case(&format!("http://{host}")),
        _ => false,
    }
}

/// The UI's file names carry no hash, so a browser must check each one again:
/// a cached old script under a new `index.html` fails its integrity check.
async fn no_cache(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// A file of the UI, or `index.html` for any other path, which the router
/// then reads.
async fn ui_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = match Ui::get(path).filter(|_| !path.is_empty()) {
        Some(file) => file,
        None => match Ui::get("index.html") {
            Some(file) => file,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The UI isn't built. Run `trunk build` in quilt-sync/ui.",
                )
                    .into_response();
            }
        },
    };
    (
        [(header::CONTENT_TYPE, file.metadata.mimetype().to_owned())],
        file.data,
    )
        .into_response()
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    // A `Result`, so the checks below answer before axum refuses a plain GET.
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if !origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if query.get("t").map(String::as_str) != Some(&*state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match ws {
        Ok(ws) => ws.on_upgrade(move |socket| session(socket, state)),
        Err(rejection) => rejection.into_response(),
    }
}

#[derive(Deserialize)]
struct Call {
    id: Value,
    cmd: String,
    #[serde(default)]
    args: Value,
}

async fn session(socket: WebSocket, state: AppState) {
    // Before the hello, so the client misses no event after it.
    let mut events = state.services.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let (replies, mut outbox) = mpsc::unbounded_channel::<String>();
    let hello = json!({
        "hello": {
            "version": state.services.app.version.to_string(),
            "desktop": state.desktop.is_some(),
            // The engine doesn't check for updates on its own yet.
            "updateAvailable": Value::Null,
        }
    });

    let writer = async move {
        if sink
            .send(Message::Text(hello.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
        loop {
            let text = tokio::select! {
                reply = outbox.recv() => match reply {
                    Some(text) => text,
                    None => break,
                },
                event = events.recv() => match event {
                    Ok((name, payload)) => json!({ "event": name, "payload": payload }).to_string(),
                    Err(RecvError::Lagged(missed)) => {
                        // It reconnects and reads again; that beats a page
                        // that silently missed events.
                        warn!("closing a client that missed {missed} events");
                        break;
                    }
                    Err(RecvError::Closed) => break,
                },
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    };

    let reader = async {
        while let Some(Ok(message)) = stream.next().await {
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => break,
                _ => continue,
            };
            match serde_json::from_str::<Call>(&text) {
                Ok(call) => {
                    let state = state.clone();
                    let replies = replies.clone();
                    tokio::spawn(async move {
                        let reply = match dispatch(
                            &state.services,
                            state.desktop.as_deref(),
                            &call.cmd,
                            call.args,
                        )
                        .await
                        {
                            Ok(ok) => json!({ "id": call.id, "ok": ok }),
                            Err(err) => json!({ "id": call.id, "err": err }),
                        };
                        let _ = replies.send(reply.to_string());
                    });
                }
                Err(err) => {
                    warn!("unreadable request: {err}");
                    // Answer the id when there is one, so the caller never hangs.
                    if let Some(id) = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|value| value.get("id").cloned())
                    {
                        let reply = json!({ "id": id, "err": format!("bad request: {err}") });
                        let _ = replies.send(reply.to_string());
                    }
                }
            }
        }
    };

    // Whichever ends first ends the session: the client left, or it fell
    // behind and was closed.
    tokio::select! {
        () = writer => {}
        () = reader => {}
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::SinkExt;
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use super::*;

    const TOKEN: &str = "0123abcd";

    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        headers
    }

    #[test]
    fn the_host_must_be_this_loopback_server() {
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
    fn the_origin_must_be_the_page_itself() {
        assert!(origin_allowed(&headers(None, "127.0.0.1:7878")));
        assert!(origin_allowed(&headers(
            Some("http://127.0.0.1:7878"),
            "127.0.0.1:7878"
        )));
        for bad in [
            "http://evil.example",
            "http://127.0.0.1:9999",
            "https://127.0.0.1:7878",
            "127.0.0.1:7878",
            "null",
        ] {
            assert!(
                !origin_allowed(&headers(Some(bad), "127.0.0.1:7878")),
                "{bad}"
            );
        }
    }

    struct Running {
        addr: SocketAddr,
        services: Arc<Services>,
        _data_dir: tempfile::TempDir,
    }

    async fn start() -> Running {
        let data_dir = tempfile::tempdir().unwrap();
        let services = Services::init(data_dir.path().to_path_buf(), semver::Version::new(1, 2, 3))
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = serve(listener, Arc::clone(&services), None, TOKEN.to_owned()).unwrap();
        Running {
            addr,
            services,
            _data_dir: data_dir,
        }
    }

    /// A raw HTTP/1.1 GET, answered with the status line and the headers.
    async fn get(addr: SocketAddr, path: &str, extra: &[(&str, &str)]) -> (u16, Vec<String>) {
        use std::fmt::Write;

        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
        for (name, value) in extra {
            let _ = write!(request, "{name}: {value}\r\n");
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        let head = response.split("\r\n\r\n").next().unwrap_or_default();
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap();
        (status, lines.map(str::to_owned).collect())
    }

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn connect(addr: SocketAddr) -> Socket {
        let request = format!("ws://{addr}/ws?t={TOKEN}")
            .into_client_request()
            .unwrap();
        tokio_tungstenite::connect_async(request).await.unwrap().0
    }

    async fn next_json(socket: &mut Socket) -> Value {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .expect("a message in time")
                .expect("the socket is open")
                .unwrap();
            if let tungstenite::Message::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_needs_no_token_and_is_never_cached() {
        let running = start().await;
        for path in ["/", "/commit?namespace=a%2Fb", "/?t=nope"] {
            let (status, headers) = get(running.addr, path, &[]).await;
            assert_ne!(status, 401, "{path}");
            assert_ne!(status, 303, "{path}");
            assert!(
                headers
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case("cache-control: no-cache")),
                "{path}"
            );
            assert!(
                !headers
                    .iter()
                    .any(|h| h.to_ascii_lowercase().starts_with("set-cookie")),
                "{path}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_socket_refuses_a_stranger_before_upgrading() {
        let running = start().await;
        let addr = running.addr;
        let port = addr.port();

        assert_eq!(get(addr, "/ws", &[]).await.0, 401);
        assert_eq!(get(addr, "/ws?t=nope", &[]).await.0, 401);
        assert_eq!(
            get(addr, "/ws?t=0123abcd", &[("Origin", "http://evil.example")])
                .await
                .0,
            403
        );
        // Over plain HTTP the right token passes the checks, and only the
        // missing upgrade is refused.
        let origin = format!("http://{addr}");
        let (status, _) = get(addr, "/ws?t=0123abcd", &[("Origin", &origin)]).await;
        assert!(status != 401 && status != 403, "{status}");
        // A cookie is no token.
        let cookie = format!("quiltsync-{port}={TOKEN}");
        assert_eq!(get(addr, "/ws", &[("Cookie", &cookie)]).await.0, 401);

        // DNS rebinding: a page on another name that resolves here.
        let mut request = format!("ws://{addr}/ws?t={TOKEN}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Host", format!("evil.example:{port}").parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
            other => panic!("expected 403, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_hears_hello_first_then_gets_replies_by_id() {
        let running = start().await;
        let mut socket = connect(running.addr).await;

        let hello = next_json(&mut socket).await;
        assert_eq!(
            hello,
            json!({"hello": {"version": "1.2.3", "desktop": false, "updateAvailable": null}})
        );

        for call in [
            json!({"id": 1, "cmd": "test_quiltignore_pattern", "args": {"pattern": "*.tmp", "path": "a.tmp"}}),
            json!({"id": 2, "cmd": "get_toasts"}),
            json!({"id": 3, "cmd": "open_directory_picker"}),
            json!({"id": 4, "cmd": "nope"}),
        ] {
            socket
                .send(tungstenite::Message::Text(call.to_string().into()))
                .await
                .unwrap();
        }
        socket
            .send(tungstenite::Message::Text(r#"{"id": 5}"#.into()))
            .await
            .unwrap();

        let mut replies = HashMap::new();
        while replies.len() < 5 {
            let reply = next_json(&mut socket).await;
            if let Some(id) = reply.get("id").and_then(Value::as_u64) {
                replies.insert(id, reply);
            }
        }
        assert_eq!(replies[&1]["ok"], json!(true));
        assert_eq!(replies[&2]["ok"], json!([]));
        assert_eq!(
            replies[&3]["err"],
            json!(crate::engine::desktop::NOT_AVAILABLE)
        );
        assert_eq!(replies[&4]["err"], json!("command nope not found"));
        assert!(
            replies[&5]["err"]
                .as_str()
                .unwrap()
                .starts_with("bad request")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_client_hears_every_event() {
        let running = start().await;
        let mut first = connect(running.addr).await;
        let mut second = connect(running.addr).await;
        next_json(&mut first).await;
        next_json(&mut second).await;

        running.services.events.emit("ping", json!({"n": 1}));

        for socket in [&mut first, &mut second] {
            loop {
                let message = next_json(socket).await;
                if message["event"] == "ping" {
                    assert_eq!(message["payload"], json!({"n": 1}));
                    break;
                }
            }
        }
    }
}

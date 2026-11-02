//! The wire contract of [`MixpanelClient`], checked against a local HTTP server.
//!
//! The server answers each connection with the next scripted reply and closes
//! it. Retry tests run on paused time, so their waits cost nothing and the gaps
//! between requests read exactly.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

use super::{MixpanelClient, SendError, WireEvent};

const TOKEN: &str = "a-token";

struct Reply {
    status: u16,
    retry_after: Option<u64>,
    body: String,
}

fn reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        retry_after: None,
        body: body.to_owned(),
    }
}

fn accepted() -> Reply {
    reply(200, r#"{"status":1,"error":null}"#)
}

fn rate_limited(retry_after: Option<u64>) -> Reply {
    Reply {
        retry_after,
        ..reply(429, "")
    }
}

#[derive(Debug)]
struct Request {
    at: Instant,
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Request {
    /// The events the body carries: `data=` and the base64 of a JSON array.
    fn events(&self) -> Vec<Value> {
        let data = self
            .body
            .strip_prefix("data=")
            .unwrap_or_else(|| panic!("body is not data=…: {}", self.body));
        let json = STANDARD.decode(data).expect("standard base64");
        serde_json::from_slice(&json).expect("a JSON array")
    }
}

struct Server {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    /// Answer with `replies` in order; past the end, repeat the last one.
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("an address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            let mut replies = replies.into_iter().peekable();
            let mut last = None;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let next = replies.next().map(Arc::new);
                if let Some(next) = next {
                    last = Some(next);
                }
                let reply = Arc::clone(last.as_ref().expect("at least one reply"));
                serve(stream, &reply, &seen).await;
            }
        });
        Self { addr, requests }
    }

    fn client(&self, secret: Option<&str>) -> MixpanelClient {
        MixpanelClient::with_base_url(TOKEN, secret, &format!("http://{}", self.addr))
    }

    fn requests(&self) -> std::sync::MutexGuard<'_, Vec<Request>> {
        self.requests.lock().expect("not poisoned")
    }

    fn gaps(&self) -> Vec<Duration> {
        self.requests()
            .windows(2)
            .map(|pair| pair[1].at - pair[0].at)
            .collect()
    }
}

async fn serve(mut stream: TcpStream, reply: &Reply, seen: &Mutex<Vec<Request>>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    seen.lock().expect("not poisoned").push(request);

    let extra = reply
        .retry_after
        .map(|secs| {
            format!(
                "Retry-After: {secs}
"
            )
        })
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {} Scripted\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        reply.status,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(reply.body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let at = Instant::now();
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&chunk[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let head = String::from_utf8(raw[..head_end].to_vec()).ok()?;
    let mut lines = head.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_owned();
    let target = start.next()?.to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();

    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while raw.len() - head_end < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8(raw[head_end..].to_vec()).ok()?;

    Some(Request {
        at,
        method,
        target,
        headers,
        body,
    })
}

fn event(name: &str) -> WireEvent {
    WireEvent {
        event: name.to_owned(),
        properties: HashMap::from([("n".to_owned(), Value::from(name))]),
    }
}

fn events(count: usize) -> Vec<WireEvent> {
    (0..count).map(|i| event(&i.to_string())).collect()
}

fn names(request: &Request) -> Vec<String> {
    request
        .events()
        .iter()
        .map(|e| e["event"].as_str().expect("a name").to_owned())
        .collect()
}

#[tokio::test]
async fn a_batch_is_one_form_post_to_track() {
    let server = Server::start(vec![accepted()]).await;

    server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect("delivered");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/track?ip=0&verbose=1");
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/x-www-form-urlencoded")
    );
    assert!(
        !request.headers.contains_key("authorization"),
        "no secret, no auth: {:?}",
        request.headers
    );
}

#[tokio::test]
async fn each_event_gains_the_token_and_the_library_properties() {
    let server = Server::start(vec![accepted()]).await;
    let mut properties = HashMap::from([
        ("host".to_owned(), json!("quilt.test")),
        ("time".to_owned(), json!(1_700_000_000)),
        // The client's own values win over the caller's.
        ("mp_lib".to_owned(), json!("not-rust")),
    ]);
    properties.insert("$insert_id".to_owned(), json!("an-id"));

    server
        .client(None)
        .send(vec![WireEvent {
            event: "package_pulled".to_owned(),
            properties,
        }])
        .await
        .expect("delivered");

    assert_eq!(
        server.requests()[0].events(),
        vec![json!({
            "event": "package_pulled",
            "properties": {
                "host": "quilt.test",
                "time": 1_700_000_000,
                "$insert_id": "an-id",
                "token": TOKEN,
                "mp_lib": "rust",
                "$lib_version": "0.3.1",
            },
        })]
    );
}

#[tokio::test]
async fn a_secret_is_sent_as_basic_auth_with_an_empty_password() {
    let server = Server::start(vec![accepted()]).await;

    server
        .client(Some("s3cret"))
        .send(vec![event("one")])
        .await
        .expect("delivered");

    assert_eq!(
        server.requests()[0]
            .headers
            .get("authorization")
            .map(String::as_str),
        Some(format!("Basic {}", STANDARD.encode("s3cret:")).as_str())
    );
}

#[tokio::test]
async fn events_go_in_chunks_of_fifty_in_order() {
    let server = Server::start(vec![accepted()]).await;

    server
        .client(None)
        .send(events(120))
        .await
        .expect("delivered");

    let requests = server.requests();
    let sizes: Vec<usize> = requests.iter().map(|r| r.events().len()).collect();
    assert_eq!(sizes, vec![50, 50, 20]);
    let sent: Vec<String> = requests.iter().flat_map(names).collect();
    let expected: Vec<String> = (0..120).map(|i| i.to_string()).collect();
    assert_eq!(sent, expected);
}

#[tokio::test]
async fn an_empty_batch_sends_nothing() {
    let server = Server::start(vec![accepted()]).await;

    server
        .client(None)
        .send(Vec::new())
        .await
        .expect("nothing to fail");

    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn a_failed_chunk_stops_the_batch() {
    let server = Server::start(vec![accepted(), reply(400, "bad"), accepted()]).await;

    let err = server
        .client(None)
        .send(events(120))
        .await
        .expect_err("the second chunk was refused");

    assert!(matches!(err, SendError::Refused(_)), "{err:?}");
    assert_eq!(server.requests().len(), 2, "the third chunk was sent");
}

/// A 2xx answer is read as JSON, and only `status` 1 is a delivery.
#[tokio::test]
async fn a_2xx_answer_is_a_delivery_only_with_status_one() {
    for (status, body) in [
        (200, r#"{"status":1}"#),
        (202, r#"{"status":1,"error":null}"#),
    ] {
        let server = Server::start(vec![reply(status, body)]).await;
        server
            .client(None)
            .send(vec![event("one")])
            .await
            .unwrap_or_else(|err| panic!("{status} {body} is a delivery: {err}"));
    }

    for body in [
        r#"{"status":0,"error":"some data fields are invalid"}"#,
        r#"{"status":0}"#,
        r#"{"status":"1"}"#,
        r#"{"error":null}"#,
        "1",
        "not json",
        "",
    ] {
        let server = Server::start(vec![reply(200, body)]).await;
        let err = server
            .client(None)
            .send(vec![event("one")])
            .await
            .expect_err(body);
        assert!(matches!(err, SendError::Refused(_)), "{body}: {err:?}");
        assert_eq!(server.requests().len(), 1, "{body} was retried");
    }
}

#[tokio::test]
async fn the_api_error_message_is_kept() {
    let server = Server::start(vec![reply(
        200,
        r#"{"status":0,"error":"some data fields are invalid"}"#,
    )])
    .await;

    let err = server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect_err("refused");

    assert!(
        err.to_string().contains("some data fields are invalid"),
        "{err}"
    );
}

/// A 4xx is a refusal and is not retried; its body is kept in the message.
#[tokio::test]
async fn a_4xx_is_a_refusal() {
    for status in [400, 401, 403, 404, 413] {
        let server = Server::start(vec![reply(status, "no such token")]).await;
        let err = server
            .client(None)
            .send(vec![event("one")])
            .await
            .expect_err("refused");
        assert!(matches!(err, SendError::Refused(_)), "{status}: {err:?}");
        assert_eq!(server.requests().len(), 1, "{status} was retried");
        if status != 413 {
            assert!(err.to_string().contains("no such token"), "{status}: {err}");
        }
    }
}

/// A status outside 2xx, 4xx and 5xx is not retried and gets no verdict.
#[tokio::test]
async fn a_3xx_without_a_location_is_unreachable() {
    let server = Server::start(vec![reply(304, "")]).await;

    let err = server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect_err("not a delivery");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_5xx_is_retried_with_backoff() {
    let server = Server::start(vec![reply(500, ""), reply(503, ""), accepted()]).await;

    server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect("delivered on the third try");

    assert_eq!(
        server.gaps(),
        vec![Duration::from_secs(1), Duration::from_secs(2)]
    );
    let bodies: Vec<String> = server.requests().iter().map(|r| r.body.clone()).collect();
    assert!(
        bodies.windows(2).all(|pair| pair[0] == pair[1]),
        "a retry sent a different body"
    );
}

#[tokio::test(start_paused = true)]
async fn a_5xx_gives_up_after_three_retries() {
    let server = Server::start(vec![reply(500, "")]).await;

    let err = server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect_err("gave up");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(
        server.gaps(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_429_waits_for_retry_after() {
    // Longer than the backoff cap: the server's wait is not capped.
    let server = Server::start(vec![rate_limited(Some(30)), accepted()]).await;

    server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect("delivered after the wait");

    assert_eq!(server.gaps(), vec![Duration::from_secs(30)]);
}

#[tokio::test(start_paused = true)]
async fn a_429_without_retry_after_backs_off() {
    let server = Server::start(vec![rate_limited(None)]).await;

    let err = server
        .client(None)
        .send(vec![event("one")])
        .await
        .expect_err("gave up");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(
        server.gaps(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_connection_is_retried_then_unreachable() {
    // Bound and dropped, so nothing listens on the port.
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind")
        .local_addr()
        .expect("an address");
    let client = MixpanelClient::with_base_url(TOKEN, None, &format!("http://{addr}"));

    let started = Instant::now();
    let err = client
        .send(vec![event("one")])
        .await
        .expect_err("nothing listens");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(started.elapsed(), Duration::from_secs(1 + 2 + 4));
}

/// A connection that closes before answering is not retried.
#[tokio::test(start_paused = true)]
async fn a_dropped_answer_is_unreachable_without_a_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("an address");
    let accepted = Arc::new(Mutex::new(0usize));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = read_request(&mut stream).await;
            *count.lock().expect("not poisoned") += 1;
            drop(stream);
        }
    });
    let client = MixpanelClient::with_base_url(TOKEN, None, &format!("http://{addr}"));

    let err = client
        .send(vec![event("one")])
        .await
        .expect_err("no answer");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(*accepted.lock().expect("not poisoned"), 1);
}

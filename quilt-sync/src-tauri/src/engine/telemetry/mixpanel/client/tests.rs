//! The wire contract of [`MixpanelClient`], checked against a local HTTP server.
//!
//! The server answers each connection with the next scripted reply and closes
//! it. Retry tests run on paused time, so their waits cost nothing.
//!
//! Paused time jumps to the next timer whenever the runtime waits. A ticking
//! task keeps each jump to a millisecond, so the client's request timeout
//! cannot fire while the local server is still answering. Gaps between
//! requests are read in whole seconds.

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

enum Reply {
    Answer {
        status: u16,
        retry_after: Option<u64>,
        body: String,
    },
    /// Read the request, then close the connection without an answer.
    Close,
    /// Answer 200, then close the connection before the body is complete.
    CutOff,
    /// Read the request, then keep the connection open without an answer.
    Silence,
}

fn reply(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        retry_after: None,
        body: body.to_owned(),
    }
}

fn accepted() -> Reply {
    reply(200, r#"{"status":1,"error":null}"#)
}

fn rate_limited(retry_after: Option<u64>) -> Reply {
    Reply::Answer {
        status: 429,
        retry_after,
        body: String::new(),
    }
}

/// See the module docs: under paused time, the clock moves in steps of a
/// millisecond while this runs.
fn tick() {
    tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
}

fn whole_seconds(duration: Duration) -> u64 {
    (duration + Duration::from_millis(500)).as_secs()
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
        tick();
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

    fn client(&self) -> MixpanelClient {
        MixpanelClient::with_base_url(TOKEN, &format!("http://{}", self.addr))
    }

    fn requests(&self) -> std::sync::MutexGuard<'_, Vec<Request>> {
        self.requests.lock().expect("not poisoned")
    }

    fn gaps(&self) -> Vec<u64> {
        self.requests()
            .windows(2)
            .map(|pair| whole_seconds(pair[1].at - pair[0].at))
            .collect()
    }
}

async fn serve(mut stream: TcpStream, reply: &Reply, seen: &Mutex<Vec<Request>>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    seen.lock().expect("not poisoned").push(request);

    let (status, retry_after, body) = match reply {
        Reply::Answer {
            status,
            retry_after,
            body,
        } => (status, retry_after, body),
        Reply::Close => return,
        Reply::CutOff => {
            let head = "HTTP/1.1 200 Scripted\r\nContent-Length: 100\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(br#"{"status""#).await;
            let _ = stream.shutdown().await;
            return;
        }
        Reply::Silence => {
            tokio::spawn(async move {
                let _open = stream;
                std::future::pending::<()>().await;
            });
            return;
        }
    };
    let extra = retry_after.map_or_default(|secs| format!("Retry-After: {secs}\r\n"));
    let head = format!(
        "HTTP/1.1 {status} Scripted\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
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
        .client()
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
        "a /track request carries no auth header: {:?}",
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
        ("token".to_owned(), json!("caller-token")),
        ("mp_lib".to_owned(), json!("caller-lib")),
        ("$lib_version".to_owned(), json!("caller-version")),
    ]);
    properties.insert("$insert_id".to_owned(), json!("an-id"));

    server
        .client()
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
                "mp_lib": "quiltsync",
                "$lib_version": env!("CARGO_PKG_VERSION"),
            },
        })]
    );
}

#[tokio::test]
async fn events_go_in_chunks_of_fifty_in_order() {
    let server = Server::start(vec![accepted()]).await;

    server.client().send(events(120)).await.expect("delivered");

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
        .client()
        .send(Vec::new())
        .await
        .expect("nothing to fail");

    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn a_failed_chunk_stops_the_batch() {
    let server = Server::start(vec![accepted(), reply(400, "bad"), accepted()]).await;

    let err = server
        .client()
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
            .client()
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
            .client()
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
        .client()
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
            .client()
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
        .client()
        .send(vec![event("one")])
        .await
        .expect_err("not a delivery");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.requests().len(), 1);
}

/// A request that cannot be built is our bug: refused, not retried.
#[tokio::test]
async fn a_request_that_cannot_be_built_is_a_refusal() {
    let client = MixpanelClient::with_base_url(TOKEN, "not a url");

    let err = client
        .send(vec![event("one")])
        .await
        .expect_err("no request");

    assert!(matches!(err, SendError::Refused(_)), "{err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_5xx_is_retried_with_backoff() {
    let server = Server::start(vec![reply(500, ""), reply(503, ""), accepted()]).await;

    server
        .client()
        .send(vec![event("one")])
        .await
        .expect("delivered on the third try");

    assert_eq!(server.gaps(), vec![1, 2]);
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
        .client()
        .send(vec![event("one")])
        .await
        .expect_err("gave up");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.gaps(), vec![1, 2, 4]);
}

#[tokio::test(start_paused = true)]
async fn a_429_waits_for_retry_after() {
    // Longer than any backoff: the server's wait wins.
    let server = Server::start(vec![rate_limited(Some(5)), accepted()]).await;

    server
        .client()
        .send(vec![event("one")])
        .await
        .expect("delivered after the wait");

    assert_eq!(server.gaps(), vec![5]);
}

/// A wait the send's 10 s cannot cover ends it at once, so the caller can
/// buffer the batch.
#[tokio::test(start_paused = true)]
async fn a_retry_after_longer_than_the_send_has_left_is_unreachable_at_once() {
    for (replies, requests, seconds) in [
        (vec![rate_limited(Some(30))], 1, 0),
        (vec![rate_limited(Some(u64::MAX))], 1, 0),
        // 1 s and 2 s of backoff leave 7 s, less than 8.
        (
            vec![reply(500, ""), reply(500, ""), rate_limited(Some(8))],
            3,
            3,
        ),
    ] {
        let server = Server::start(replies).await;
        let started = Instant::now();

        let err = server
            .client()
            .send(vec![event("one")])
            .await
            .expect_err("not worth the wait");

        assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
        assert_eq!(server.requests().len(), requests);
        assert_eq!(whole_seconds(started.elapsed()), seconds);
    }
}

/// Later sends wait out a Retry-After that ended a send, without a request.
#[tokio::test(start_paused = true)]
async fn a_retry_after_that_ended_a_send_holds_the_next_sends() {
    let server = Server::start(vec![rate_limited(Some(30)), accepted()]).await;
    let client = server.client();

    let err = client.send(vec![event("one")]).await.expect_err("too long");
    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");

    tokio::time::advance(Duration::from_secs(29)).await;
    let err = client.send(vec![event("two")]).await.expect_err("held");
    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.requests().len(), 1);

    tokio::time::advance(Duration::from_secs(2)).await;
    client
        .send(vec![event("three")])
        .await
        .expect("delivered after the wait");
    assert_eq!(server.requests().len(), 2);
}

/// The same holds when the wait comes with the last retry.
#[tokio::test(start_paused = true)]
async fn a_retry_after_on_the_last_retry_holds_the_next_sends() {
    // 1 s, 2 s and 4 s of backoff leave 3 s, enough for a 2 s wait.
    let mut replies: Vec<Reply> = (0..3).map(|_| reply(500, "")).collect();
    replies.extend([rate_limited(Some(2)), accepted()]);
    let server = Server::start(replies).await;
    let client = server.client();

    let err = client
        .send(vec![event("one")])
        .await
        .expect_err("out of retries");
    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");

    let err = client.send(vec![event("two")]).await.expect_err("held");
    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.requests().len(), 4);

    tokio::time::advance(Duration::from_secs(3)).await;
    client
        .send(vec![event("three")])
        .await
        .expect("delivered after the wait");
    assert_eq!(server.requests().len(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_429_without_retry_after_backs_off() {
    let server = Server::start(vec![rate_limited(None)]).await;

    let err = server
        .client()
        .send(vec![event("one")])
        .await
        .expect_err("gave up");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert_eq!(server.gaps(), vec![1, 2, 4]);
}

#[tokio::test(start_paused = true)]
async fn a_refused_connection_is_retried_then_unreachable() {
    // Bound and dropped, so nothing listens on the port.
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind")
        .local_addr()
        .expect("an address");
    let client = MixpanelClient::with_base_url(TOKEN, &format!("http://{addr}"));
    tick();

    let started = Instant::now();
    let err = client
        .send(vec![event("one")])
        .await
        .expect_err("nothing listens");

    assert!(matches!(err, SendError::Unreachable(_)), "{err:?}");
    assert!(err.to_string().contains("after 3 retries"), "{err}");
    // Windows takes about 2 s to refuse, so each try may end at its 3 s
    // timeout instead; either way the backoff comes on top.
    assert!(whole_seconds(started.elapsed()) >= 1 + 2 + 4);
}

/// A connection that closes before answering is retried like a refused one.
#[tokio::test(start_paused = true)]
async fn a_dropped_answer_is_retried() {
    let server = Server::start(vec![Reply::Close, accepted()]).await;

    server
        .client()
        .send(vec![event("one")])
        .await
        .expect("delivered on the second try");

    assert_eq!(server.gaps(), vec![1]);
}

/// A 2xx whose body is cut off gave no verdict, so it is retried.
#[tokio::test(start_paused = true)]
async fn a_cut_off_body_is_retried() {
    let server = Server::start(vec![Reply::CutOff, accepted()]).await;

    server
        .client()
        .send(vec![event("one")])
        .await
        .expect("delivered on the second try");

    assert_eq!(server.gaps(), vec![1]);
}

/// A request with no answer for 3 s is given up and retried.
#[tokio::test(start_paused = true)]
async fn a_silent_request_times_out_and_is_retried() {
    let server = Server::start(vec![Reply::Silence, accepted()]).await;

    server
        .client()
        .send(vec![event("one")])
        .await
        .expect("delivered on the second try");

    assert_eq!(server.gaps(), vec![3 + 1]);
}

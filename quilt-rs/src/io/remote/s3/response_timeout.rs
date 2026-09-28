//! A bound on how long S3 may stay silent once a request is sent.
//!
//! The SDK already bounds the connect (3.1 s), and its stalled-stream
//! protection fails a request or response body that moves under 1 byte per
//! second for a 5 s grace period. Nothing bounds the wait between the last
//! byte of the request and the first byte of the response, so a server that
//! takes a request and never answers holds the call forever.
//!
//! The SDK's own `read_timeout` does not fit: its clock starts before the
//! request body is sent, so it would cut off any upload slower than it. This
//! wrapper starts the clock only once the body is fully sent (or right away
//! for a request with no body), and stops it at the response headers. It
//! never bounds how long a transfer takes, only how long S3 stays silent.

use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use aws_sdk_s3::config::HttpClient;
use aws_sdk_s3::config::RuntimeComponents;
use aws_sdk_s3::config::RuntimeComponentsBuilder;
use aws_sdk_s3::config::SharedHttpClient;
use aws_smithy_http_client::ConnectorBuilder;
use aws_smithy_http_client::proxy::ProxyConfig;
use aws_smithy_http_client::tls;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::connector_metadata::ConnectorMetadata;
use aws_smithy_runtime_api::client::http::HttpConnector;
use aws_smithy_runtime_api::client::http::HttpConnectorFuture;
use aws_smithy_runtime_api::client::http::HttpConnectorSettings;
use aws_smithy_runtime_api::client::http::SharedHttpConnector;
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_types::body::SdkBody;
use aws_smithy_types::config_bag::ConfigBag;
use aws_types::SdkConfig;
use tokio::sync::oneshot;

/// How long S3 may take to start its response once the whole request is sent.
///
/// The same as botocore's default read timeout. The slowest call is
/// `CompleteMultipartUpload`: S3 sends its headers once it starts the work,
/// then whitespace until the result. For 10,000 parts with SHA-256 checksums
/// (S3's maximum) the headers took about 6 s, so one value serves every call.
pub(super) const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// The S3 client for `config`, with [`RESPONSE_TIMEOUT`].
pub(super) fn s3_client(config: &SdkConfig) -> aws_sdk_s3::Client {
    s3_client_with_timeout(config, RESPONSE_TIMEOUT)
}

fn s3_client_with_timeout(config: &SdkConfig, timeout: Duration) -> aws_sdk_s3::Client {
    let conf = aws_sdk_s3::config::Builder::from(config)
        .http_client(ResponseTimeoutClient {
            inner: default_https_client(),
            timeout,
        })
        .build();
    aws_sdk_s3::Client::from_conf(conf)
}

/// The client the SDK builds when none is given: hyper 1 over rustls with
/// aws-lc, proxies read from the environment. `aws-smithy-runtime` keeps its
/// own constructor private, so this repeats it: `default_https_client` in
/// `src/client/http.rs` of aws-smithy-runtime 1.15.0, for a behavior version
/// of 2025-08-07 or later. Recheck it against that function when the SDK is
/// bumped.
fn default_https_client() -> SharedHttpClient {
    aws_smithy_http_client::Builder::new().build_with_connector_fn(|settings, components| {
        let mut builder = ConnectorBuilder::default().tls_provider(tls::Provider::Rustls(
            tls::rustls_provider::CryptoMode::AwsLc,
        ));
        builder.set_connector_settings(settings.cloned());
        if let Some(components) = components {
            builder.set_sleep_impl(components.sleep_impl());
        }
        builder.set_proxy_config(Some(ProxyConfig::from_env()));
        builder.build()
    })
}

#[derive(Debug)]
struct ResponseTimeoutClient {
    inner: SharedHttpClient,
    timeout: Duration,
}

impl HttpClient for ResponseTimeoutClient {
    fn http_connector(
        &self,
        settings: &HttpConnectorSettings,
        components: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(ResponseTimeoutConnector {
            inner: self.inner.http_connector(settings, components),
            timeout: self.timeout,
        })
    }

    fn validate_base_client_config(
        &self,
        runtime_components: &RuntimeComponentsBuilder,
        cfg: &ConfigBag,
    ) -> Result<(), BoxError> {
        self.inner
            .validate_base_client_config(runtime_components, cfg)
    }

    fn connector_metadata(&self) -> Option<ConnectorMetadata> {
        self.inner.connector_metadata()
    }
}

#[derive(Debug)]
struct ResponseTimeoutConnector {
    inner: SharedHttpConnector,
    timeout: Duration,
}

impl HttpConnector for ResponseTimeoutConnector {
    fn call(&self, mut request: HttpRequest) -> HttpConnectorFuture {
        let (sent_tx, sent) = oneshot::channel::<()>();
        let body = std::mem::replace(request.body_mut(), SdkBody::taken());
        if http_body::Body::is_end_stream(&body) {
            *request.body_mut() = body;
            drop(sent_tx);
        } else {
            *request.body_mut() = SdkBody::from_body_1_x(NotifyWhenSent {
                inner: body,
                sent: Some(sent_tx),
            });
        }
        let response = self.inner.call(request);
        let timeout = self.timeout;
        HttpConnectorFuture::new(async move {
            // `sent` resolves once the body is done: finished, failed or
            // dropped. Either way nothing more is being sent.
            let silence = async move {
                let _ = sent.await;
                tokio::time::sleep(timeout).await;
            };
            tokio::select! {
                response = response => response,
                () = silence => Err(ConnectorError::timeout(
                    format!("S3 sent no response within {timeout:?} of the request").into(),
                )),
            }
        })
    }
}

/// A request body that drops `sent` once it has nothing more to send.
struct NotifyWhenSent {
    inner: SdkBody,
    sent: Option<oneshot::Sender<()>>,
}

impl http_body::Body for NotifyWhenSent {
    type Data = bytes::Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let frame = std::task::ready!(Pin::new(&mut self.inner).poll_frame(cx));
        if !matches!(frame, Some(Ok(_))) || self.inner.is_end_stream() {
            self.sent = None;
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        http_body::Body::is_end_stream(&self.inner)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::Body::size_hint(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::future::Future;
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::task::Context;
    use std::task::Poll;
    use std::time::Instant;

    use aws_config::BehaviorVersion;
    use aws_config::retry::RetryConfig;
    use aws_credential_types::Credentials;
    use aws_credential_types::provider::SharedCredentialsProvider;
    use aws_sdk_s3::error::SdkError;
    use aws_sdk_s3::primitives::ByteStream;
    use aws_sdk_s3::types::CompletedMultipartUpload;
    use aws_sdk_s3::types::CompletedPart;
    use aws_smithy_types::body::SdkBody;
    use aws_types::region::Region;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio::net::TcpStream;

    use crate::io::remote::s3::tests::read_http_request;

    /// Short enough to keep the tests quick, long enough that a loaded CI
    /// runner does not trip it by accident.
    const TIMEOUT: Duration = Duration::from_secs(1);

    /// A step of a slow exchange: far below [`TIMEOUT`].
    const TICK: Duration = Duration::from_millis(200);

    /// How many [`TICK`]s each slow-but-alive exchange takes.
    const STEPS: usize = 15;

    /// How long each slow-but-alive exchange lasts: well past [`TIMEOUT`].
    const SLOW: Duration = Duration::from_millis(200 * STEPS as u64);

    /// Longer than any call under test should take, so a call that hangs
    /// fails the test instead of hanging it.
    const HANG: Duration = Duration::from_secs(20);

    /// Serve every connection with `handle`, which gets the socket right after
    /// the connection is accepted.
    async fn spawn_endpoint<F, Fut>(handle: F) -> SocketAddr
    where
        F: Fn(TcpStream) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(handle(stream));
            }
        });
        addr
    }

    /// Read the request, then never answer, keeping the connection open.
    async fn silent(mut stream: TcpStream) {
        if read_http_request(&mut stream).await.is_ok() {
            std::future::pending::<()>().await;
        }
    }

    /// A client for `addr` with a [`TIMEOUT`] bound and no retries, so each
    /// call is exactly one attempt.
    fn client(addr: SocketAddr) -> aws_sdk_s3::Client {
        let config = SdkConfig::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(SharedCredentialsProvider::new(Credentials::new(
                "AK", "SK", None, None, "test",
            )))
            .endpoint_url(format!("http://{addr}"))
            .retry_config(RetryConfig::disabled())
            .build();
        let s3 = s3_client_with_timeout(&config, TIMEOUT);
        let conf = s3.config().to_builder().force_path_style(true).build();
        aws_sdk_s3::Client::from_conf(conf)
    }

    /// Run `call`, failing the test if it has not finished within [`HANG`].
    async fn timed<T>(call: impl Future<Output = T>) -> (T, Duration) {
        let start = Instant::now();
        let out = tokio::time::timeout(HANG, call)
            .await
            .expect("the call hung: nothing bounded the wait");
        (out, start.elapsed())
    }

    fn assert_timed_out<E, R>(
        result: Result<impl std::fmt::Debug, SdkError<E, R>>,
        elapsed: Duration,
    ) where
        E: std::fmt::Debug,
        R: std::fmt::Debug,
    {
        let err = result.expect_err("a silent server must fail the call");
        let is_timeout = matches!(&err, SdkError::DispatchFailure(e) if e.is_timeout());
        assert!(is_timeout, "expected a timeout, got: {err:?}");
        assert!(
            elapsed >= TIMEOUT && elapsed < TIMEOUT * 3,
            "timed out after {elapsed:?}, not about {TIMEOUT:?}"
        );
    }

    /// `HeadObject` (and `GetObject`, `CreateMultipartUpload`) send no body,
    /// so the wait starts as soon as the request goes out.
    #[test(tokio::test)]
    async fn silent_server_fails_a_bodiless_call() {
        let addr = spawn_endpoint(silent).await;

        let (result, elapsed) = timed(client(addr).head_object().bucket("b").key("k").send()).await;

        assert_timed_out(result, elapsed);
    }

    /// `PutObject` and `UploadPart`: the server takes the whole body, then
    /// goes silent. Stalled-stream protection stops watching once the body is
    /// sent, so this wait was unbounded.
    #[test(tokio::test)]
    async fn silent_server_fails_an_upload_after_its_body() {
        let addr = spawn_endpoint(silent).await;

        let (result, elapsed) = timed(Box::pin(
            client(addr)
                .put_object()
                .bucket("b")
                .key("k")
                .body(ByteStream::from_static(b"payload"))
                .send(),
        ))
        .await;

        assert_timed_out(result, elapsed);
    }

    /// A request body that yields `chunks` chunks of `chunk` bytes, one every
    /// [`TICK`]: an upload over a slow link.
    struct SlowBody {
        chunk: usize,
        chunks_left: usize,
        tick: Pin<Box<tokio::time::Sleep>>,
    }

    impl SlowBody {
        fn new(chunk: usize, chunks: usize) -> Self {
            Self {
                chunk,
                chunks_left: chunks,
                tick: Box::pin(tokio::time::sleep(TICK)),
            }
        }
    }

    impl http_body::Body for SlowBody {
        type Data = bytes::Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            if self.chunks_left == 0 {
                return Poll::Ready(None);
            }
            std::task::ready!(self.tick.as_mut().poll(cx));
            self.chunks_left -= 1;
            let next = tokio::time::Instant::now() + TICK;
            self.tick.as_mut().reset(next);
            Poll::Ready(Some(Ok(http_body::Frame::data(bytes::Bytes::from(
                vec![b'x'; self.chunk],
            )))))
        }

        fn is_end_stream(&self) -> bool {
            self.chunks_left == 0
        }

        fn size_hint(&self) -> http_body::SizeHint {
            http_body::SizeHint::with_exact((self.chunk * self.chunks_left) as u64)
        }
    }

    /// Read the request, then answer with `head` and an empty body.
    async fn answer(mut stream: TcpStream, head: &'static str) {
        if read_http_request(&mut stream).await.is_ok() {
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    }

    /// An upload whose body takes longer than the timeout to send, and that
    /// never stops moving, must not be cut off.
    #[test(tokio::test)]
    async fn slow_steady_upload_succeeds() {
        let addr =
            spawn_endpoint(|stream| answer(stream, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"))
                .await;
        let chunks = STEPS;
        let body = SlowBody::new(1024, chunks);
        let length = i64::try_from(1024 * chunks).unwrap();

        let (result, elapsed) = timed(Box::pin(
            client(addr)
                .put_object()
                .bucket("b")
                .key("k")
                .content_length(length)
                .body(ByteStream::new(SdkBody::from_body_1_x(body)))
                .send(),
        ))
        .await;

        result.expect("a slow upload that keeps moving must succeed");
        assert!(
            elapsed >= SLOW.saturating_sub(TICK),
            "the body was not slow: {elapsed:?}"
        );
    }

    /// Answer with `head`, then send `body` one byte every [`TICK`].
    async fn trickle(mut stream: TcpStream, head: String, body: Vec<u8>) {
        if read_http_request(&mut stream).await.is_err() {
            return;
        }
        let _ = stream.write_all(head.as_bytes()).await;
        for byte in body {
            tokio::time::sleep(TICK).await;
            if stream.write_all(&[byte]).await.is_err() {
                return;
            }
        }
        let _ = stream.shutdown().await;
    }

    /// A download whose body takes longer than the timeout, and that never
    /// stops moving, must not be cut off.
    #[test(tokio::test)]
    async fn slow_steady_download_succeeds() {
        let chunks = STEPS;
        let body = vec![b'x'; chunks];
        let expected = body.clone();
        let addr = spawn_endpoint(move |stream| {
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            trickle(stream, head, body.clone())
        })
        .await;

        let (result, elapsed) = timed(async {
            let out = client(addr)
                .get_object()
                .bucket("b")
                .key("k")
                .send()
                .await?;
            Ok::<_, Box<dyn std::error::Error>>(out.body.collect().await?.into_bytes())
        })
        .await;

        let bytes = result.expect("a slow download that keeps moving must succeed");
        assert_eq!(bytes.as_ref(), expected.as_slice());
        assert!(
            elapsed >= SLOW.saturating_sub(TICK),
            "the body was not slow: {elapsed:?}"
        );
    }

    /// S3 answers a `CompleteMultipartUpload` with `200 OK` headers and the
    /// XML declaration as soon as it starts, then sends whitespace while it
    /// works, then the result. That can take minutes; it must not be cut off.
    #[test(tokio::test)]
    async fn slow_complete_multipart_upload_succeeds() {
        const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
        const RESULT: &str = "<CompleteMultipartUploadResult><Bucket>b</Bucket>\
             <Key>k</Key><ETag>\"e\"</ETag></CompleteMultipartUploadResult>";
        let spaces = STEPS;
        let addr = spawn_endpoint(move |mut stream| async move {
            if read_http_request(&mut stream).await.is_err() {
                return;
            }
            let length = DECLARATION.len() + spaces + RESULT.len();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\n\
                 Content-Length: {length}\r\n\r\n{DECLARATION}"
            );
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for _ in 0..spaces {
                tokio::time::sleep(TICK).await;
                if stream.write_all(b" ").await.is_err() {
                    return;
                }
            }
            let _ = stream.write_all(RESULT.as_bytes()).await;
            let _ = stream.shutdown().await;
        })
        .await;

        let (result, elapsed) = timed(
            client(addr)
                .complete_multipart_upload()
                .bucket("b")
                .key("k")
                .upload_id("u")
                .multipart_upload(
                    CompletedMultipartUpload::builder()
                        .parts(CompletedPart::builder().part_number(1).e_tag("e").build())
                        .build(),
                )
                .send(),
        )
        .await;

        let out = result.expect("a slow completion that keeps sending must succeed");
        assert_eq!(out.e_tag(), Some("\"e\""));
        assert!(
            elapsed >= SLOW.saturating_sub(TICK),
            "the completion was not slow: {elapsed:?}"
        );
    }
}

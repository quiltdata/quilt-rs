//!
//! Wraps operations with remote storage. Primarily S3.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::RequestId;
use aws_sdk_s3::operation::RequestIdExt;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use tokio_stream::Stream;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use crate::object_hash::ObjectHash;
use quilt_uri::Host;
use quilt_uri::S3Uri;

pub mod client;
mod host;
mod object;
mod s3;
mod workflow;

/// Render an S3 SDK error into a short, diagnosable string.
///
/// Three tiers, in order of preference:
///
/// 1. **Service error with a code**: `"<ErrorCode>: <message> (x-amz-request-id: …)"`.
///    The normal case — S3 returned a structured XML error body and the
///    SDK parsed out a code / message (`ExpiredToken`, `AccessDenied`, …).
/// 2. **Service or response error with no code**: `"HTTP <status> (no error body; x-amz-request-id: …, x-amz-id-2: …)"`.
///    Happens when S3 returns 4xx/5xx with an empty or unparseable body.
///    Without this branch the message collapses to just `"Unknown"`, and
///    the only diagnostic signal (the AWS request IDs that support can
///    trace) gets dropped on the floor.
/// 3. **Transport / construction / timeout**: the SDK's own
///    `DisplayErrorContext` renderer — there's no raw response to mine.
///
/// Without the helper, wrapped S3 errors surface as long
/// `service error: unhandled error ... (ServiceError { … huge Debug … })`
/// strings — users paste those into bug reports and the actionable info
/// (error code, request id) gets lost in response-header noise.
pub(super) fn describe_sdk_error<E>(err: SdkError<E>) -> String
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let request_id = err.request_id().unwrap_or("-").to_string();
    let extended_id = err.extended_request_id().unwrap_or("-").to_string();

    let service_head = err.as_service_error().and_then(|svc| {
        // `ProvideErrorMetadata::code` / `message` collide with
        // `ProvideErrorKind::code` on the generated error types — spell the
        // trait out explicitly rather than relying on method-resolution.
        let code = ProvideErrorMetadata::code(svc);
        let msg = ProvideErrorMetadata::message(svc);
        match (code, msg) {
            (Some(c), Some(m)) if !m.is_empty() => Some(format!("{c}: {m}")),
            (Some(c), _) => Some(c.to_string()),
            (None, Some(m)) if !m.is_empty() => Some(m.to_string()),
            (None, _) => None,
        }
    });
    if let Some(head) = service_head {
        return format!("{head} (x-amz-request-id: {request_id})");
    }

    if let Some(raw) = err.raw_response() {
        let status = raw.status().as_u16();
        return format!(
            "HTTP {status} (no error body; x-amz-request-id: {request_id}, x-amz-id-2: {extended_id})"
        );
    }

    DisplayErrorContext(err).to_string()
}

/// Recover a `LoginError::Required` that the AWS SDK wrapped on its way out.
///
/// Credentials are vended *lazily*, inside the SDK's credential provider, so
/// "you are signed out" does not reach a call site as an auth error — the SDK
/// boxes it as a `CredentialsError`, wraps that in a `ConnectorError`, and
/// hands back an `SdkError::DispatchFailure` that is indistinguishable at the
/// boundary from a transport fault. Left alone it flattens through
/// [`describe_sdk_error`]'s transport tier into a diagnostic string, and every
/// consumer that branches on error kind sees generic storage trouble: the
/// desktop watcher retries it silently as a transient, and the UI has nothing
/// to offer the user but the wrap chain.
///
/// So each call path that can hit an unauthenticated vend asks this first, and
/// re-raises the typed error instead of classifying it as S3 trouble.
///
/// **This walks `source()`, not the SDK's error types.** Downcasting the
/// intermediate `ConnectorError` / `CredentialsError` would pin us to shapes
/// that are internal to the SDK; the source chain is a `std` contract. What we
/// still depend on — and what no shape assertion could pin either — is that the
/// SDK *preserves* our boxed error as a source rather than stringifying it. A
/// dependency bump can sever that silently, so the pin for this is behavioural:
/// [`login_required_survives_the_sdk_wrap`](#tests) builds the real wrap and
/// asserts recovery, and fails if the chain is ever broken.
pub(super) fn recover_login_required(err: &(dyn std::error::Error + 'static)) -> Option<Error> {
    let mut current = Some(err);
    while let Some(e) = current {
        if let Some(Error::Login(LoginError::Required(host))) = e.downcast_ref::<Error>() {
            return Some(Error::Login(LoginError::Required(host.clone())));
        }
        current = e.source();
    }
    None
}

pub use crate::workflow::{
    WORKFLOWS_CONFIG_KEY, WorkflowInfo, WorkflowIntent, WorkflowSchemaUris, WorkflowsConfig,
};
pub use client::HttpClient;
pub use host::{HostChecksums, HostConfig, fetch_host_config};
pub use s3::RemoteS3;
pub(crate) use workflow::{
    entry_view, fetch_workflows_config, resolve_workflow_from_config, validate_workflow,
    validate_workflow_against_current_config, validate_workflow_with_config,
};
pub use workflow::{fetch_workflow_rules, fetch_workflows_config_for_bucket, resolve_workflow};

// Mock remote is available during testing, or to downstream crates via the
// `testing` feature.
#[cfg(any(test, feature = "testing"))]
pub mod mocks;

pub struct RemoteObjectStream {
    pub body: ByteStream,
    pub uri: S3Uri,
}

pub type StreamObjectChunk = Vec<Res<Object>>;

pub type StreamItem = Res<StreamObjectChunk>;

pub trait ObjectsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> ObjectsStream for T {}

/// This trait encapsulates the S3 operations that Quilt needs to perform.
pub trait Remote {
    /// Checks if object exists
    fn exists(&self, host: Option<&Host>, s3_uri: &S3Uri)
    -> impl Future<Output = Res<bool>> + Send;

    /// Fetches the objects contents as a `ByteStream`
    fn get_object_stream(
        &self,
        host: Option<&Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<RemoteObjectStream>> + Send;

    // Makes a head request and resolves the final versioned URL
    fn resolve_url(
        &self,
        host: Option<&Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<S3Uri>> + Send;

    /// Upload file. Just that
    fn put_object(
        &self,
        host: Option<&Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> impl Future<Output = Res>;

    /// Upload file and request checkum from S3
    fn upload_file(
        &self,
        host_config: &HostConfig,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> impl Future<Output = Res<(S3Uri, ObjectHash)>>;

    /// Fetch host configuration from the given host
    fn host_config(&self, host: Option<&Host>) -> impl Future<Output = Res<HostConfig>> + Send;

    /// Verify that a bucket exists and is addressable on S3. Used for
    /// pre-flight validation when a user sets a remote, so a typo in
    /// the bucket name fails at save time rather than surfacing later
    /// as an opaque error during push. Does not require auth — AWS's
    /// HEAD-bucket endpoint returns the region for any existing bucket
    /// regardless of permissions.
    fn verify_bucket(&self, bucket: &str) -> impl Future<Output = Res> + Send;

    /// Drop any cached clients (and their in-memory credentials) for the
    /// given `host`, or for all hosts when `None`. Lets callers invalidate
    /// credentials immediately on logout instead of waiting for them to
    /// expire. Default no-op: only stateful remotes such as [`RemoteS3`]
    /// cache clients, so mocks and other impls need no change.
    fn clear_client_cache(&self, host: Option<&Host>) {
        let _ = host;
    }
}

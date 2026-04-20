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

use crate::checksum::ObjectHash;
use crate::uri::Host;
use crate::uri::S3Uri;
use crate::Res;

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

pub use client::HttpClient;
pub use host::{fetch_host_config, HostChecksums, HostConfig};
pub use s3::RemoteS3;
pub use workflow::resolve_workflow;

#[cfg(test)]
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
    fn exists(&self, host: &Option<Host>, s3_uri: &S3Uri)
        -> impl Future<Output = Res<bool>> + Send;

    /// Fetches the objects contents as a `ByteStream`
    fn get_object_stream(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<RemoteObjectStream>> + Send;

    // Makes a head request and resolves the final versioned URL
    fn resolve_url(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<S3Uri>> + Send;

    /// Upload file. Just that
    fn put_object(
        &self,
        host: &Option<Host>,
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
    fn host_config(&self, host: &Option<Host>) -> impl Future<Output = Res<HostConfig>> + Send;
}

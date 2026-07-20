//! Common hash trait for all checksum implementations
use std::future::Future;

use multihash::Multihash;
use tokio::io::AsyncRead;

use crate::object_hash::error::Error;

/// This trait ensures all hash types provide consistent access to the underlying multihash
pub trait Hash {
    /// Get the inner multihash
    fn multihash(&self) -> &Multihash<256>;

    /// Get the algorithm code
    fn algorithm(&self) -> u64 {
        self.multihash().code()
    }

    /// Get the digest bytes
    fn digest(&self) -> &[u8] {
        self.multihash().digest()
    }

    /// Calculate hash from an async reader of `length` bytes.
    ///
    /// `length` is required by chunked algorithms to derive multipart chunk
    /// boundaries; algorithms that hash the whole stream ignore it. The caller
    /// supplies the length (e.g. from file metadata) since a reader does not
    /// carry one.
    fn from_reader<R: AsyncRead + Unpin + Send>(
        reader: R,
        length: u64,
    ) -> impl Future<Output = Result<Self, Error>> + Send
    where
        Self: Sized;
}

//! Common hash trait for all checksum implementations
use std::future::Future;

use multihash::Multihash;
use tokio::fs::File;

use crate::Res;

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

    /// Calculate hash from a file
    fn from_file(file: File) -> impl Future<Output = Res<Self>> + Send
    where
        Self: Sized;
}

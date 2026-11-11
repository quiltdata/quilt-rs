//! Commands over the [record](crate::engine::record).

use quilt_uri::Namespace;

use crate::engine::record::Entry;
use crate::engine::services::Services;

/// Every entry kept, oldest first: the window's read on mount and on each
/// `record-changed`.
pub async fn get_record(s: &Services) -> Result<Vec<Entry>, String> {
    Ok(s.record.entries().await)
}

/// The reader has looked at the entries up to `up_to`: all of them from the
/// inbox, or `namespace`'s from its package page.
pub async fn mark_read(
    s: &Services,
    up_to: u64,
    namespace: Option<Namespace>,
) -> Result<(), String> {
    s.record.mark_read(up_to, namespace).await;
    Ok(())
}

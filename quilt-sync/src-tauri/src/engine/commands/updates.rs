//! The command over the [update feed](crate::engine::updates).

use crate::engine::services::Services;
use crate::engine::telemetry::prelude::*;
use crate::engine::updates::UpdateInfo;

/// A newer `QuiltSync`, if the feed offers one.
pub async fn check_for_update(s: &Services) -> Result<Option<UpdateInfo>, String> {
    // The UI ignores a failed check so users are not nagged on every launch,
    // which leaves the log as the only place the failure shows.
    s.updates
        .check()
        .await
        .inspect_err(|e| warn!("update check failed: {e}"))
}

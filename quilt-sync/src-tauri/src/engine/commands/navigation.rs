//! Commands over the [pending navigations](crate::engine::navigation).

use crate::engine::services::Services;

/// Where page `page` is to go, if the engine sent it somewhere; once.
pub async fn take_navigation(s: &Services, page: String) -> Result<Option<String>, String> {
    Ok(s.navigations.take(&page))
}

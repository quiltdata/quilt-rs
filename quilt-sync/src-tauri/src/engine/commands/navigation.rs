//! Commands over the [pending navigations](crate::engine::navigation).

use crate::engine::navigation::Navigation;
use crate::engine::services::Services;

/// Where page `page` is to go, if the engine sent it somewhere, until it says
/// it went.
pub async fn pending_navigation(s: &Services, page: String) -> Result<Option<Navigation>, String> {
    Ok(s.navigations.pending(&page))
}

/// Page `page` followed navigation `id`.
pub async fn navigation_done(s: &Services, page: String, id: String) -> Result<(), String> {
    s.navigations.done(&page, &id);
    Ok(())
}

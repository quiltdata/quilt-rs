//! Commands over the [toast centre](crate::engine::toast).

use crate::engine::services::Services;
use crate::engine::toast::Toast;

/// Every undismissed toast, oldest first.
///
/// The hydration read: the `toast` event alone reaches only a window that was
/// already open, while the centre holds what was posted while none was.
pub async fn get_toasts(s: &Services) -> Result<Vec<Toast>, String> {
    let center = &*s.toasts;
    Ok(center.live().await)
}

/// Dismiss one toast. Unknown ids succeed — two windows closing the same toast
/// is ordinary, not an error.
pub async fn dismiss_toast(s: &Services, id: u64) -> Result<(), String> {
    let center = &*s.toasts;
    center.dismiss(id).await;
    Ok(())
}

//! Commands over the [toast centre](crate::toast).

use crate::toast::Toast;
use crate::toast::ToastCenter;

/// Every undismissed toast, oldest first.
///
/// The hydration read: the `toast` event alone reaches only a window that was
/// already open, while the centre holds what was posted while none was.
#[tauri::command]
pub async fn get_toasts(center: tauri::State<'_, ToastCenter>) -> Result<Vec<Toast>, String> {
    Ok(center.live().await)
}

/// Dismiss one toast. Unknown ids succeed — two windows closing the same toast
/// is ordinary, not an error.
#[tauri::command]
pub async fn dismiss_toast(center: tauri::State<'_, ToastCenter>, id: u64) -> Result<(), String> {
    center.dismiss(id).await;
    Ok(())
}

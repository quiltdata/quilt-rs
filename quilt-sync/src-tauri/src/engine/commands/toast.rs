//! Commands over the [toast centre](crate::engine::toast).

use crate::engine::services::Services;
use crate::engine::toast::TOAST_DISMISSED_EVENT;
use crate::engine::toast::Toast;

/// Every undismissed toast, oldest first.
///
/// The hydration read: the `toast` event alone reaches only a window that was
/// already open, while the centre holds what was posted while none was.
pub async fn get_toasts(s: &Services) -> Result<Vec<Toast>, String> {
    let center = &*s.toasts;
    Ok(center.live().await)
}

/// Dismiss one toast, and tell every client to drop it. Unknown ids succeed —
/// two windows closing the same toast is ordinary, not an error.
pub async fn dismiss_toast(s: &Services, id: u64) -> Result<(), String> {
    let center = &*s.toasts;
    if center.dismiss(id).await {
        s.events.emit(TOAST_DISMISSED_EVENT, id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::*;
    use crate::engine::toast::ToastDraft;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dismissal_reaches_every_client() {
        let data_dir = tempfile::tempdir().unwrap();
        let s = Services::init(data_dir.path().to_path_buf(), Version::new(0, 0, 0))
            .await
            .unwrap();
        let id = s.toasts.post(ToastDraft::default()).await;
        let mut events = s.events.subscribe();

        dismiss_toast(&s, id).await.unwrap();

        loop {
            let (name, payload) = events.recv().await.unwrap();
            if name == TOAST_DISMISSED_EVENT {
                assert_eq!(payload, serde_json::json!(id));
                break;
            }
        }
    }
}

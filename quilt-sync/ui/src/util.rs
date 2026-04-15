use std::future::Future;

use leptos::prelude::*;

use crate::components::layout::Notification;

/// Create a busy-guarded async action handler.
///
/// Returns `(busy_signal, click_handler)`. The handler guards against
/// double-clicks, optionally locks the UI, runs the command, and shows
/// a success/error notification. On success it calls `on_done` (e.g.
/// to trigger a refetch or navigate).
pub fn make_action<F, Fut>(
    command: F,
    notification: RwSignal<Option<Notification>>,
    ui_locked: Option<RwSignal<bool>>,
    on_done: impl Fn() + 'static + Clone,
) -> (RwSignal<bool>, impl Fn(leptos::ev::MouseEvent) + 'static)
where
    F: Fn() -> Fut + 'static + Clone,
    Fut: Future<Output = Result<String, String>> + 'static,
{
    let busy = RwSignal::new(false);
    let handler = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        if let Some(ui_locked) = ui_locked {
            ui_locked.set(true);
        }
        let command = command.clone();
        let on_done = on_done.clone();
        leptos::task::spawn_local(async move {
            match command().await {
                Ok(msg) => {
                    // Don't reset `busy` here — the refetch triggered by
                    // `on_done` will destroy/rebuild the component, clearing
                    // the signal. Resetting early would briefly re-enable the
                    // button and allow duplicate clicks.
                    if let Some(ui_locked) = ui_locked {
                        ui_locked.set(false);
                    }
                    notification.set(Some(Notification::Success(msg)));
                    on_done();
                }
                Err(e) => {
                    // Don't call on_done — nothing changed on the server.
                    if let Some(ui_locked) = ui_locked {
                        ui_locked.set(false);
                    }
                    notification.set(Some(Notification::Error(e)));
                    busy.set(false);
                }
            }
        });
    };
    (busy, handler)
}

/// Validate that `value` is a syntactically valid hostname
/// (two or more dot-separated labels, each starting and ending with
/// an ASCII alphanumeric character, with hyphens allowed in the middle).
pub fn is_valid_hostname(value: &str) -> bool {
    let mut count = 0u32;
    for label in value.split('.') {
        if !is_valid_label(label) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn is_valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    for unit in UNITS {
        if value < 1000.0 {
            if *unit == "B" {
                return format!("{value} {unit}");
            }
            return format!("{value:.2} {unit}");
        }
        value /= 1000.0;
    }
    format!("{value:.2} EB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_hostnames() {
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("sub.example.com"));
        assert!(is_valid_hostname("my-host.example.co.uk"));
        assert!(is_valid_hostname("a.b"));
        assert!(is_valid_hostname("123.456"));
        assert!(is_valid_hostname("a-1.b-2.c-3"));
    }

    #[test]
    fn rejects_single_label() {
        assert!(!is_valid_hostname("localhost"));
        assert!(!is_valid_hostname("example"));
    }

    #[test]
    fn rejects_empty_and_dot_only() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("."));
        assert!(!is_valid_hostname(".."));
    }

    #[test]
    fn rejects_leading_trailing_hyphen() {
        assert!(!is_valid_hostname("-example.com"));
        assert!(!is_valid_hostname("example-.com"));
        assert!(!is_valid_hostname("example.-com"));
        assert!(!is_valid_hostname("example.com-"));
    }

    #[test]
    fn rejects_invalid_characters() {
        assert!(!is_valid_hostname("exam ple.com"));
        assert!(!is_valid_hostname("exam_ple.com"));
        assert!(!is_valid_hostname("example!.com"));
    }

    #[test]
    fn rejects_empty_labels() {
        assert!(!is_valid_hostname(".example.com"));
        assert!(!is_valid_hostname("example..com"));
        assert!(!is_valid_hostname("example.com."));
    }

    #[test]
    fn format_size_zero() {
        assert_eq!(format_size(0), "0 B");
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1500), "1.50 kB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(2_500_000), "2.50 MB");
    }
}

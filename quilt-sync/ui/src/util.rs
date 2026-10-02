use std::future::Future;

use leptos::prelude::*;
use quilt_uri::S3PackageUri;

use crate::components::layout::Notification;

/// Format the catalog HTTPS URL for a package URI, if its `catalog`
/// host is set. Used to power "Open in catalog" buttons.
pub fn catalog_url(uri: &S3PackageUri) -> Option<String> {
    uri.display_for_catalog().ok().map(|u| u.to_string())
}

/// Format the catalog HTTPS URL for an individual entry inside a package.
/// Returns `None` if the parent URI has no catalog host. Call from the
/// "Open in catalog" click handler so we only build the URL on click,
/// not for every row at render time.
pub fn entry_catalog_url(pkg_uri: &S3PackageUri, filename: &str) -> Option<String> {
    // Existence check: skip the clone when there's no catalog, since
    // the downstream `display_for_catalog()` would return None anyway.
    // `?` propagates the None; the bound `&Host` is discarded.
    pkg_uri.catalog.as_ref()?;
    let entry_uri = S3PackageUri {
        path: Some(std::path::PathBuf::from(filename)),
        ..pkg_uri.clone()
    };
    catalog_url(&entry_uri)
}

/// The `quilt+s3` handle of an installed package, from the parts the main
/// page's payload carries.
///
/// `Latest`, with no hash: a copied address points at the package, not at the
/// revision the copier happens to hold — a recipient asking for a pinned
/// revision would have to be given one deliberately.
///
/// A catalog host that does not parse is dropped rather than failing the whole
/// address, the catalog being the one optional part of it.
#[must_use]
pub fn package_uri(
    bucket: &str,
    namespace: &quilt_uri::Namespace,
    catalog: Option<&str>,
) -> S3PackageUri {
    S3PackageUri {
        catalog: catalog.and_then(|host| host.parse().ok()),
        bucket: bucket.to_string(),
        namespace: namespace.clone(),
        revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
        path: None,
    }
}

/// The same handle, pointed at one file inside the package — the form the feed's
/// Copy action puts on the clipboard.
#[must_use]
pub fn file_uri(package: &S3PackageUri, path: &str) -> S3PackageUri {
    S3PackageUri {
        path: Some(std::path::PathBuf::from(path)),
        ..package.clone()
    }
}

/// Stringified catalog host, if set.
pub fn host_str(uri: &S3PackageUri) -> Option<String> {
    uri.catalog.as_ref().map(std::string::ToString::to_string)
}

/// Bucket name, treating an empty string as "unset" — matches the
/// historical `current_bucket` field semantics.
pub fn bucket_str(uri: &S3PackageUri) -> Option<String> {
    Some(uri.bucket.clone()).filter(|b| !b.is_empty())
}

/// Create a busy-guarded async action handler.
///
/// Returns `(busy_signal, click_handler)`. The handler guards against
/// double-clicks, optionally locks the UI, runs the command, and shows
/// a success/error notification. On success it calls `on_done` (e.g.
/// to trigger a refetch or navigate).
///
/// `busy` is **not** reset on success — `on_done` is expected to trigger
/// a re-render that destroys the component (and its signal). If `on_done`
/// does not rebuild the component, the button will remain disabled.
/// What the page's notification slot should hold after a command succeeded.
///
/// An empty message means the command had nothing to say. `None`, not
/// `Some("")`: `Layout` raises a full-screen dismiss overlay for as long as the
/// slot holds anything, which would dim the app behind a blank.
fn success_notification(msg: String) -> Option<Notification> {
    (!msg.is_empty()).then_some(Notification::Success(msg))
}

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
                    notification.set(success_notification(msg));
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

/// The line shown for an autosync pause whose cause is a role denial
/// (`reason = "roleDenied"`), given the role name the event carried.
///
/// Shared by the list row and the detail banner so both surfaces say the same
/// thing. The generic paused guidance ("push manually to resume") is wrong
/// here — a manual push under the same role is denied too — so this names the
/// only fix there is. The wording matches the roster's `no_access_reason`,
/// and it stays a neutral fact: switching to a narrow role on purpose is a
/// legitimate thing to have done.
///
/// `None` means the backend could not resolve the role name, not that no role
/// is active.
pub fn role_denied_hint(role: Option<&str>) -> String {
    match role {
        Some(role) if !role.is_empty() => format!(
            "Current role {role} has no access to this bucket. Switch role to resume autosync."
        ),
        _ => "The active role has no access to this bucket. Switch role to resume autosync."
            .to_string(),
    }
}

/// The tooltip on a commit affordance the active role cannot use, given the
/// backend's `no_access_reason`; `None` when the role can reach the bucket and
/// the affordance stays live.
///
/// Committing is not offline work: the workflow quality gate reads the
/// bucket's `.quilt/workflows/config.yml` before any manifest is written, so
/// under a denied role neither Commit nor Commit and Push can succeed. Both
/// the commit page and the installed-package page word it this way, and the
/// sentence continues the reason the roster already states rather than
/// restating it — same neutral fact, same named role, plus the only fix.
pub fn commit_denied_hint(no_access_reason: Option<&str>) -> Option<String> {
    let reason = no_access_reason.filter(|reason| !reason.is_empty())?;
    Some(format!("{reason}. Switch role to commit."))
}

/// Why committing is blocked when there is no session, and what fixes it.
/// Never a role switch — the role was never the problem. Keyed on whether
/// there is a deployment: one can be signed into, ambient AWS credentials can
/// only be fixed in the file.
pub fn commit_no_session_hint(no_session: bool, no_session_host: Option<&str>) -> Option<String> {
    if !no_session {
        return None;
    }
    Some(match no_session_host.filter(|host| !host.is_empty()) {
        Some(host) => format!("Not signed in to {host}. Sign in to commit."),
        None => "No usable AWS credentials. Update ~/.aws/credentials to commit.".to_string(),
    })
}

/// A logical size, in decimal units: whole bytes under 1 kB, else one decimal
/// in the largest unit that keeps the figure under 1000 — "999 B", "44.0 kB",
/// "3.4 MB", up to "18.4 EB" for `u64::MAX`. The one formatter for sizes on
/// every page. The space is a no-break one: in a narrow pane "1.2" ending a
/// line with "TB" starting the next reads as two figures.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes}\u{a0}B");
    }
    // In u128, so `n * 10 + scale / 2` cannot overflow for any u64: the
    // largest is under 2^68. Integers, so no float rounds "999.95" either way.
    let n = u128::from(bytes);
    let mut scale = 1000u128;
    let mut tenths = 0;
    let mut unit = UNITS[0];
    for next in UNITS {
        unit = next;
        tenths = (n * 10 + scale / 2) / scale;
        // Rounding up into the next unit's "1000.0" moves on to that unit.
        if tenths < 10_000 {
            break;
        }
        scale *= 1000;
    }
    format!("{}.{}\u{a0}{unit}", tenths / 10, tenths % 10)
}

/// A count, grouped in thousands: "140,000" is read, "140000" is counted.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(text: &str) -> quilt_uri::Namespace {
        quilt_uri::Namespace::try_from(text).expect("a namespace")
    }

    /// The one claim a copied address makes: the string. Round-tripped rather
    /// than only compared, because the point of the `quilt+s3` form is that
    /// something else parses it back — `S3PackageUri::try_from` is the reader on
    /// the other end of a paste.
    #[test]
    fn a_file_address_names_its_bucket_package_path_and_catalog() {
        let package = package_uri("team-bucket", &ns("user/plate-07"), Some("quilt.test"));
        let file = file_uri(&package, "runs/a/one.csv");

        assert_eq!(
            file.display(),
            "quilt+s3://team-bucket#package=user/plate-07&path=runs/a/one.csv&catalog=quilt.test"
        );
        assert_eq!(
            S3PackageUri::try_from(file.display().as_str()).expect("it parses back"),
            file
        );
    }

    /// No catalog is not no address: the bucket and the package are what a
    /// `quilt+s3` URI needs, and a deployment the payload never named must not
    /// stop a copy.
    #[test]
    fn a_package_with_no_catalog_still_has_an_address() {
        let package = package_uri("team-bucket", &ns("user/plate-07"), None);

        assert_eq!(
            file_uri(&package, "one.csv").display(),
            "quilt+s3://team-bucket#package=user/plate-07&path=one.csv"
        );
    }

    /// The revision is deliberately absent — `Latest`, which `display` renders
    /// as nothing. A copied address points at the package, not at whatever
    /// revision the copier happened to hold.
    #[test]
    fn a_copied_address_pins_no_revision() {
        let package = package_uri("b", &ns("user/p"), None);

        let address = package.display();
        let package_spec = address
            .split("#package=")
            .nth(1)
            .expect("a package in the fragment");

        assert_eq!(
            package_spec, "user/p",
            "the package spec carries no `:tag` and no `@hash`: {address}"
        );
    }

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
    fn role_denied_hint_names_the_role_and_the_fix() {
        assert_eq!(
            role_denied_hint(Some("ReadOnly")),
            "Current role ReadOnly has no access to this bucket. \
             Switch role to resume autosync."
        );
    }

    #[test]
    fn role_denied_hint_without_a_name_still_offers_the_fix() {
        // The backend sends no message when it could not resolve the name.
        // An empty string is the same case, defensively.
        let unnamed = "The active role has no access to this bucket. \
                       Switch role to resume autosync.";
        assert_eq!(role_denied_hint(None), unnamed);
        assert_eq!(role_denied_hint(Some("")), unnamed);
    }

    #[test]
    fn commit_denied_hint_continues_the_reason_with_the_fix() {
        assert_eq!(
            commit_denied_hint(Some("Current role ReadOnly has no access to this bucket")),
            Some(
                "Current role ReadOnly has no access to this bucket. Switch role to commit."
                    .to_string()
            )
        );
        // The unnamed wording (the role query itself failed) reads the same way.
        assert_eq!(
            commit_denied_hint(Some("The active role has no access to this bucket")),
            Some(
                "The active role has no access to this bucket. Switch role to commit.".to_string()
            )
        );
    }

    #[test]
    fn commit_denied_hint_is_absent_on_a_readable_bucket() {
        assert_eq!(commit_denied_hint(None), None);
        // An empty reason is not a denial either, defensively.
        assert_eq!(commit_denied_hint(Some("")), None);
    }

    /// The words as read, with the no-break spaces shown as spaces.
    fn plain(words: &str) -> String {
        words.replace('\u{a0}', " ")
    }

    #[test]
    fn sizes_take_one_decimal_in_the_largest_unit() {
        assert_eq!(plain(&format_size(0)), "0 B");
        assert_eq!(plain(&format_size(512)), "512 B");
        assert_eq!(plain(&format_size(999)), "999 B");
        assert_eq!(plain(&format_size(1500)), "1.5 kB");
        assert_eq!(plain(&format_size(44_000)), "44.0 kB");
        assert_eq!(plain(&format_size(114_100)), "114.1 kB");
        assert_eq!(plain(&format_size(1_900_000)), "1.9 MB");
        assert_eq!(plain(&format_size(2_500_000)), "2.5 MB");
        assert_eq!(plain(&format_size(999_960)), "1.0 MB");
        assert_eq!(plain(&format_size(1_200_000_000_000)), "1.2 TB");
    }

    #[test]
    fn the_unit_is_held_to_its_number_by_a_no_break_space() {
        assert_eq!(format_size(512), "512\u{a0}B");
        assert_eq!(format_size(3_400_000), "3.4\u{a0}MB");
    }

    /// No u64 overflows the arithmetic, and rounding at each unit's edge moves
    /// on to the next unit rather than reading "1000.0".
    #[test]
    fn sizes_hold_at_the_unit_edges_and_at_the_top() {
        assert_eq!(plain(&format_size(1_000)), "1.0 kB");
        assert_eq!(plain(&format_size(999_949)), "999.9 kB");
        assert_eq!(plain(&format_size(999_950)), "1.0 MB");
        assert_eq!(plain(&format_size(999_949_999)), "999.9 MB");
        assert_eq!(plain(&format_size(999_950_000)), "1.0 GB");
        assert_eq!(plain(&format_size(999_950_000_000)), "1.0 TB");
        assert_eq!(plain(&format_size(999_949_999_999_999)), "999.9 TB");
        assert_eq!(plain(&format_size(999_950_000_000_000)), "1.0 PB");
        assert_eq!(plain(&format_size(999_950_000_000_000_000)), "1.0 EB");
        assert_eq!(plain(&format_size(u64::MAX - 1)), "18.4 EB");
        assert_eq!(plain(&format_size(u64::MAX)), "18.4 EB");
    }

    #[test]
    fn counts_carry_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(4_312), "4,312");
        assert_eq!(thousands(140_000), "140,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}

/// The sentence naming what an incoming revision brings, or `None` when it
/// brings no new file.
///
/// One wording for both surfaces that say it — the package-list row and the
/// package screen's behind banner — because a user who sees both should read
/// the same sentence. They differ only in `max_named`, the row having less
/// room.
///
/// Names some and counts the rest rather than truncating silently: the point is
/// that the user can see *which* files, and a bare count is what they already
/// had.
#[must_use]
pub fn incoming_files(added: &[String], max_named: usize) -> Option<String> {
    if added.is_empty() {
        return None;
    }
    let plural = if added.len() == 1 { "" } else { "s" };
    let named = added
        .iter()
        .take(max_named)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let rest = added.len().saturating_sub(max_named);
    let tail = if rest > 0 {
        format!(" and {rest} more")
    } else {
        String::new()
    };
    Some(format!("{} new file{plural}: {named}{tail}", added.len()))
}

#[cfg(test)]
mod incoming_tests {
    use super::Notification;
    use super::incoming_files;
    use super::success_notification;

    // `#[wasm_bindgen_test]`, not `#[test]`: this crate's only runner is the
    // wasm one, which executes nothing else. These need no DOM.
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn a_revision_that_adds_nothing_says_nothing() {
        assert_eq!(incoming_files(&[], 3), None);
    }

    #[wasm_bindgen_test]
    fn one_file_is_named_in_the_singular() {
        assert_eq!(
            incoming_files(&["qc/summary.csv".to_owned()], 3).unwrap(),
            "1 new file: qc/summary.csv"
        );
    }

    #[wasm_bindgen_test]
    fn a_long_list_names_some_and_counts_the_rest() {
        let added: Vec<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(
            incoming_files(&added, 2).unwrap(),
            "4 new files: a, b and 2 more"
        );
        // The same list with room for all of them names all of them.
        assert_eq!(
            incoming_files(&added, 4).unwrap(),
            "4 new files: a, b, c, d"
        );
    }

    /// A reported pull returns an empty success message, and the slot must hold
    /// nothing at all: `Layout` raises a full-screen dismiss overlay whenever
    /// the slot is occupied, so `Some("")` would dim the app behind a blank.
    #[wasm_bindgen_test]
    fn an_empty_success_message_leaves_the_slot_empty() {
        assert!(success_notification(String::new()).is_none());
    }

    #[wasm_bindgen_test]
    fn a_message_that_says_something_still_reaches_the_slot() {
        match success_notification("Successfully pulled package acme/demo".to_owned()) {
            Some(Notification::Success(msg)) => {
                assert_eq!(msg, "Successfully pulled package acme/demo");
            }
            _ => panic!("a non-empty message must reach the slot as a success"),
        }
    }
}

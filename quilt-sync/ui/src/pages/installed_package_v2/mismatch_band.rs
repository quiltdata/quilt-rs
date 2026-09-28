//! The band a deep link leaves when it asked for a revision other than the
//! installed one.
//!
//! Opening a `quilt+s3` link for another revision switches nothing: the deep
//! link lands here with the requested hash and its remote in the address
//! (`routes::RevisionMismatch`). The band says so, naming both revisions by
//! commit message with the hash on hover, because the readers are bench
//! scientists and a hash says nothing to them.
//!
//! # The address carries it, so every address the page builds keeps it
//!
//! The band stands while the address carries the mismatch. The mismatch is one
//! of the deep link's outcomes (`routes::DeepLinkOutcome`), and the page's
//! `carrying` keeps whichever outcome the address has on every address it
//! builds for the same package — entering Resolve, leaving it, and the
//! replacement a success or a normalised address makes — so a mode entered on
//! the page must not end the band.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use crate::commands;
use crate::kit::{Banner, BannerVariant, PackageState};
use crate::routes::RevisionMismatch;

/// The requested revision's commit message, looked up by hash on its own
/// remote, for the package at `namespace`.
pub(crate) type RevisionMessage =
    fn(RevisionMismatch, String) -> Pin<Box<dyn Future<Output = Result<Option<String>, String>>>>;

pub(super) fn app_revision_message(
    mismatch: RevisionMismatch,
    namespace: String,
) -> Pin<Box<dyn Future<Output = Result<Option<String>, String>>>> {
    let RevisionMismatch {
        hash,
        bucket,
        catalog,
    } = mismatch;
    Box::pin(commands::get_revision_message(
        bucket, namespace, hash, catalog,
    ))
}

/// The requested side's message, looked up once per address.
///
/// On the page rather than in the band: every re-read rebuilds the band, and
/// a lookup owned by it would drop back to the hash on each watcher event and
/// ask the remote again for an answer that cannot have changed. An answer for
/// an address the reader has since left is dropped, as the outcome band drops
/// a stale result.
pub(super) fn requested_message(
    asked: Memo<Option<RevisionMismatch>>,
    namespace: Memo<String>,
    lookup: RevisionMessage,
) -> RwSignal<Option<String>> {
    let message = RwSignal::new(None);
    Effect::new(move |_| {
        let mismatch = asked.get();
        let ns = namespace.get();
        message.set(None);
        let Some(mismatch) = mismatch else {
            return;
        };
        leptos::task::spawn_local(async move {
            let answer = lookup(mismatch.clone(), ns.clone()).await;
            let current = asked.try_get_untracked().flatten().as_ref() == Some(&mismatch)
                && namespace.try_get_untracked().as_ref() == Some(&ns);
            if current {
                message.try_set(answer.ok().flatten());
            }
        });
    });
    message
}

/// The band itself, drawn after the outcome and the pause.
///
/// # Warning, and no dismiss
///
/// It reports no failure, so it does not interrupt. It stands while the
/// address carries the mismatch: leaving the page is what ends it, and a
/// dismiss would only hide a thing still true of the address.
///
/// # The line follows the page's own state
///
/// v1 keys its line on the upstream state, which cannot tell a conflict from a
/// divergence. This page has resolved the state already, so the line reads the
/// same answer the header draws.
///
/// # Nothing when the requested revision is installed
///
/// *Get latest* can install exactly the revision the link asked for, and the
/// re-read that follows still has the query in the address, so the band
/// compares hashes rather than trusting the address to say there is a mismatch.
pub(super) fn mismatch_band(
    asked: Memo<Option<RevisionMismatch>>,
    requested: Signal<Option<String>>,
    installed: commands::CurrentRevisionData,
    state: &PackageState,
) -> AnyView {
    let line = state_line(state);
    let installed = StoredValue::new(installed);
    (move || {
        let RevisionMismatch { hash, .. } = asked.get()?;
        let commands::CurrentRevisionData {
            hash: installed_hash,
            message: installed_message,
            ..
        } = installed.get_value();
        if installed_hash == hash {
            return None;
        }
        Some(view! {
            <Banner variant=BannerVariant::Warning>
                <strong>"Requested version:"</strong>
                " "
                {move || identity(requested.get().as_deref(), &hash)}
                <br />
                <strong>"Installed version:"</strong>
                " "
                {identity(installed_message.as_deref(), &installed_hash)}
                <br />
                {line}
            </Banner>
        })
    })
    .into_any()
}

/// A revision by its message, or by its short hash when it has none, with the
/// full hash on hover either way — so an identity is never blank.
fn identity(message: Option<&str>, hash: &str) -> AnyView {
    let named = match message.map(str::trim) {
        Some(message) if !message.is_empty() => message.to_string(),
        _ => hash.chars().take(8).collect(),
    };
    view! { <span title=hash.to_string()>{named}</span> }.into_any()
}

/// Every line opens on the same clause, then says what the reader is looking
/// at. A state that cannot reach the remote — no session, an expired sign-in,
/// a denied role, no bucket, an unexplained pause, a kind this build does not
/// know — says only that it cannot be checked.
fn state_line(state: &PackageState) -> &'static str {
    match state {
        PackageState::Behind => {
            "The requested version isn't installed on this computer. \
             You're seeing the version you have."
        }
        PackageState::PendingChanges { .. }
        | PackageState::PendingCommit
        | PackageState::Unpublished => {
            "The requested version isn't installed on this computer. \
             You have local changes that aren't on the remote yet."
        }
        PackageState::Diverged => {
            "The requested version isn't installed on this computer. \
             Your local version has diverged from the remote — resolve that below."
        }
        PackageState::PullConflict { .. } => {
            "The requested version isn't installed on this computer. \
             Your changes conflict with the newer version — publish them, then resolve."
        }
        PackageState::Latest => {
            "The requested version isn't installed on this computer. \
             You have the latest version installed, and that's what's shown."
        }
        PackageState::NoSession { .. }
        | PackageState::SignInExpired { .. }
        | PackageState::RoleDenied { .. }
        | PackageState::NoRemote
        | PackageState::Paused
        | PackageState::Unknown => {
            "The requested version isn't installed on this computer, \
             and the remote can't be checked right now."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PackageScreen, PageRead, ResolveCommands};
    use super::*;
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms, unmount_earlier};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const INSTALLED: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const REQUESTED: &str = "c41d8f02aa55bb66cc77dd88ee99ff00c41d8f02aa55bb66cc77dd88ee99ff00";
    const PLAIN: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified";

    fn mismatch_query() -> String {
        format!("&mismatch={REQUESTED}&mrbucket=quilt-lab&mrcatalog=https%3A%2F%2Fopen.quilt.bio")
    }

    /// The page's fixture at `INSTALLED`, in `state`, with `message`.
    fn data(state: PackageState, message: Option<&str>) -> commands::PackagePageData {
        let mut page = super::super::tests::page_data();
        page.header.state = state;
        page.context.revision.hash = INSTALLED.to_string();
        page.context.revision.message = message.map(ToString::to_string);
        page
    }

    type Read = Pin<Box<dyn Future<Output = Result<commands::PackagePageData, String>>>>;
    type Lookup = Pin<Box<dyn Future<Output = Result<Option<String>, String>>>>;

    /// What a lookup was asked: the mismatch, and the package's namespace.
    type Asked = (RevisionMismatch, String);

    thread_local! {
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static LOOKUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static RELEASED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        static ASKED: std::cell::RefCell<Option<Asked>> = const { std::cell::RefCell::new(None) };
    }

    fn settled(_: String) -> Read {
        READS.set(READS.get() + 1);
        Box::pin(async { Ok(data(PackageState::Latest, Some("Initial upload"))) })
    }

    fn unmessaged(_: String) -> Read {
        READS.set(READS.get() + 1);
        Box::pin(async { Ok(data(PackageState::Latest, None)) })
    }

    fn diverged(_: String) -> Read {
        READS.set(READS.get() + 1);
        let mut d = data(PackageState::Diverged, Some("Initial upload"));
        d.context.resolve = Some(commands::ResolveData::Compared {
            published_message: Some("Theirs".to_string()),
            differing: vec!["plate/a.csv".to_string()],
            unpublished: 1,
            uncommitted: 0,
        });
        Box::pin(async move { Ok(d) })
    }

    /// The requested revision itself is installed.
    fn installed_as_asked(_: String) -> Read {
        READS.set(READS.get() + 1);
        let mut d = data(PackageState::Latest, Some("Re-run plate 7"));
        d.context.revision.hash = REQUESTED.to_string();
        Box::pin(async move { Ok(d) })
    }

    /// The installed revision first, then the requested one, as *Get latest*
    /// leaves it when the latest is the revision the link asked for.
    fn then_as_asked(address: String) -> Read {
        if READS.get() == 0 {
            settled(address)
        } else {
            installed_as_asked(address)
        }
    }

    /// Answers once the test releases it, recording what it was asked.
    fn held(mismatch: RevisionMismatch, namespace: String) -> Lookup {
        LOOKUPS.set(LOOKUPS.get() + 1);
        ASKED.set(Some((mismatch, namespace)));
        Box::pin(async {
            while !RELEASED.get() {
                sleep_ms(5).await;
            }
            Ok(Some("Re-run plate 7".to_string()))
        })
    }

    fn refused(_: RevisionMismatch, _: String) -> Lookup {
        LOOKUPS.set(LOOKUPS.get() + 1);
        Box::pin(async { Err("NoSuchKey".to_string()) })
    }

    async fn screen(address: &str, read: PageRead, lookup: RevisionMessage) -> web_sys::Element {
        unmount_earlier();
        READS.set(0);
        LOOKUPS.set(0);
        ASKED.set(None);
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
        let el = mount(move || {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                view! {
                                    <PackageScreen
                                        read=read
                                        resolving=ResolveCommands::app()
                                        revision_message=lookup
                                    />
                                }
                            }
                        />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    fn search() -> String {
        web_sys::window().unwrap().location().search().unwrap()
    }

    fn title(el: &web_sys::HtmlElement) -> Option<String> {
        el.get_attribute("title")
    }

    #[wasm_bindgen_test]
    async fn no_mismatch_no_band() {
        let el = screen(PLAIN, settled, held).await;

        element_saying(&el, "Revisions you have (1)");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("Requested version"),
            "markup was {}",
            el.inner_html()
        );
        assert_eq!(LOOKUPS.get(), 0, "nothing to look up");
    }

    /// Two phases: the installed side from the read at once, the requested side
    /// by its short hash until its own remote answers, then by its message.
    #[wasm_bindgen_test]
    async fn the_band_names_both_revisions_and_resolves_the_requested_one() {
        RELEASED.set(false);
        let el = screen(&format!("{PLAIN}{}", mismatch_query()), settled, held).await;

        element_saying(&el, "Requested version:");
        element_saying(&el, "Installed version:");
        let installed = element_saying(&el, "Initial upload");
        assert_eq!(title(&installed).as_deref(), Some(INSTALLED));
        let pending = element_saying(&el, &REQUESTED[..8]);
        assert_eq!(title(&pending).as_deref(), Some(REQUESTED));
        assert_eq!(
            ASKED.take(),
            Some((
                RevisionMismatch {
                    hash: REQUESTED.to_string(),
                    bucket: "quilt-lab".to_string(),
                    catalog: Some("https://open.quilt.bio".to_string()),
                },
                "team/dataset".to_string(),
            )),
            "looked up on the requested revision's own remote"
        );

        RELEASED.set(true);
        sleep_ms(50).await;

        let requested = element_saying(&el, "Re-run plate 7");
        assert_eq!(title(&requested).as_deref(), Some(REQUESTED));
    }

    /// An identity is never blank: no message, or a lookup that failed, names
    /// the revision by its short hash.
    #[wasm_bindgen_test]
    async fn a_revision_with_no_message_is_named_by_its_short_hash() {
        let el = screen(&format!("{PLAIN}{}", mismatch_query()), unmessaged, refused).await;

        assert_eq!(
            title(&element_saying(&el, &INSTALLED[..8])).as_deref(),
            Some(INSTALLED)
        );
        assert_eq!(
            title(&element_saying(&el, &REQUESTED[..8])).as_deref(),
            Some(REQUESTED)
        );
        assert_eq!(LOOKUPS.get(), 1);
    }

    /// It stands while the address carries the mismatch, so there is nothing
    /// to dismiss.
    #[wasm_bindgen_test]
    async fn the_band_has_no_dismiss() {
        let el = screen(&format!("{PLAIN}{}", mismatch_query()), settled, held).await;

        let band = element_saying(&el, "Requested version:")
            .closest("[role=status]")
            .unwrap()
            .expect("the band");
        assert!(
            band.query_selector("button").unwrap().is_none(),
            "markup was {}",
            band.inner_html()
        );
    }

    /// A re-read rebuilds the slot; the band comes back with it, and the
    /// requested side is not looked up again, so it does not fall back to the
    /// hash on every watcher event.
    #[wasm_bindgen_test]
    async fn the_band_stands_through_a_re_read() {
        let el = screen(&format!("{PLAIN}{}", mismatch_query()), settled, held).await;
        element_saying(&el, "Re-run plate 7");

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.get(), 2, "re-read");
        element_saying(&el, "Re-run plate 7");
        element_saying(&el, "Initial upload");
        assert_eq!(LOOKUPS.get(), 1, "looked up once per address");
    }

    /// The address still asks for a revision, but it is the installed one:
    /// there is no mismatch to report.
    #[wasm_bindgen_test]
    async fn no_band_when_the_requested_revision_is_installed() {
        let el = screen(
            &format!("{PLAIN}{}", mismatch_query()),
            installed_as_asked,
            held,
        )
        .await;

        element_saying(&el, "Revisions you have (1)");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("Requested version"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// *Get latest* installs exactly the requested revision; the re-read that
    /// follows keeps the query in the address, and the band goes.
    #[wasm_bindgen_test]
    async fn the_band_goes_when_a_re_read_installs_the_requested_revision() {
        let el = screen(&format!("{PLAIN}{}", mismatch_query()), then_as_asked, held).await;
        element_saying(&el, "Requested version:");

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.get(), 2, "re-read");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("Requested version"),
            "markup was {}",
            el.inner_html()
        );
        assert!(
            search().contains(&format!("mismatch={REQUESTED}")),
            "address left alone"
        );
    }

    /// Entering the mode and leaving it keep the query, and the band with it:
    /// a mode entered on the page must not end the band.
    #[wasm_bindgen_test]
    async fn entering_and_leaving_resolve_keep_the_mismatch() {
        let plain = format!("{PLAIN}{}", mismatch_query());
        let el = screen(&plain, diverged, held).await;

        button_saying(&el, "Resolve").click();
        sleep_ms(50).await;
        assert_eq!(
            search(),
            format!(
                "?namespace=team%2Fdataset&filter=unmodified&resolve=1{}",
                mismatch_query()
            )
        );
        element_saying(&el, "Requested version:");

        let back = el
            .query_selector(&format!("a[href='{plain}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("the back link keeps it; markup was {}", el.inner_html()));
        back.unchecked_into::<web_sys::HtmlElement>().click();
        sleep_ms(50).await;
        assert_eq!(
            search(),
            format!(
                "?namespace=team%2Fdataset&filter=unmodified{}",
                mismatch_query()
            )
        );
        element_saying(&el, "Requested version:");
    }

    /// A `resolve=1` the package cannot honour is replaced, and the
    /// replacement keeps the mismatch.
    #[wasm_bindgen_test]
    async fn a_normalised_address_keeps_the_mismatch() {
        let _el = screen(
            &format!("{PLAIN}&resolve=1{}", mismatch_query()),
            settled,
            held,
        )
        .await;

        assert_eq!(
            search(),
            format!(
                "?namespace=team%2Fdataset&filter=unmodified{}",
                mismatch_query()
            )
        );
    }

    /// One line per group of the page's own states, each opening on the same
    /// clause.
    #[test]
    fn each_state_group_has_its_line() {
        const OPENS: &str = "The requested version isn't installed on this computer";
        let behind = format!("{OPENS}. You're seeing the version you have.");
        let local = format!("{OPENS}. You have local changes that aren't on the remote yet.");
        let diverged = format!(
            "{OPENS}. Your local version has diverged from the remote — resolve that below."
        );
        let conflict = format!(
            "{OPENS}. Your changes conflict with the newer version — publish them, then resolve."
        );
        let latest =
            format!("{OPENS}. You have the latest version installed, and that's what's shown.");
        let unchecked = format!("{OPENS}, and the remote can't be checked right now.");
        let host = Some("open.quilt.bio".to_string());

        for (state, line) in [
            (PackageState::Behind, &behind),
            (PackageState::PendingChanges { files: 2 }, &local),
            (PackageState::PendingCommit, &local),
            (PackageState::Unpublished, &local),
            (PackageState::Diverged, &diverged),
            (
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string()],
                },
                &conflict,
            ),
            (PackageState::Latest, &latest),
            (PackageState::NoSession { host: host.clone() }, &unchecked),
            (PackageState::SignInExpired { host }, &unchecked),
            (PackageState::RoleDenied { role: None }, &unchecked),
            (PackageState::NoRemote, &unchecked),
            (PackageState::Paused, &unchecked),
            (PackageState::Unknown, &unchecked),
        ] {
            assert_eq!(state_line(&state), line.as_str(), "for {state:?}");
        }
    }
}

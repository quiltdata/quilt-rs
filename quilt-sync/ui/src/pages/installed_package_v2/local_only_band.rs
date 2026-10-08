//! The band a deep link leaves when it named a package this computer has with
//! no remote.
//!
//! The link names a revision on an S3 bucket, and the package here is linked to
//! none, so there is nothing to check the link's revision against. The install
//! answers local-only, the deep link lands here with `localOnly=1` in the address
//! (`routes::DeepLinkOutcome::LocalOnly`), and the band says why the reader sees
//! what they have rather than what they asked for.
//!
//! # Explains only
//!
//! No button: the header already offers *Choose S3 bucket* for a package with
//! no remote, and the link's bucket is not taken as the answer — a link to a
//! package is not a decision about where this copy publishes.
//!
//! # Carried as the mismatch is
//!
//! The flag is one of the deep link's outcomes, as a mismatch is, and goes
//! through the same carry (the page's `carrying`), so entering Resolve, leaving
//! it and the normalised address all keep it.

use leptos::prelude::*;

use crate::kit::{Banner, BannerVariant, PackageState};

/// The band itself, drawn in the mismatch band's place.
///
/// # Warning, and no dismiss
///
/// It reports no failure, so it does not interrupt, and it stands while the
/// address carries the flag, as the mismatch band does.
///
/// # Nothing once the package has a remote
///
/// Choosing a bucket re-reads the page with the flag still in the address. The
/// band follows the page's own state rather than the address, so the reason it
/// gives is gone once the state is no longer [`PackageState::NoRemote`].
pub(super) fn local_only_band(local_only: Memo<bool>, state: &PackageState) -> AnyView {
    if *state != PackageState::NoRemote {
        return ().into_any();
    }
    (move || {
        local_only.get().then(|| {
            view! {
                <Banner variant=BannerVariant::Warning>
                    "Your copy of this package isn't linked to an S3 bucket, so the link's \
                     version can't be checked. You're seeing the version you have."
                </Banner>
            }
        })
    })
    .into_any()
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::super::{PackageScreen, PageRead, ResolveCommands};
    use super::*;
    use crate::commands;
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms, unmount_earlier};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const PLAIN: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified";
    const LINE: &str = "Your copy of this package isn't linked to an S3 bucket, \
                        so the link's version can't be checked. You're seeing the version you have.";

    fn data(state: PackageState) -> commands::PackagePageData {
        let mut page = super::super::tests::page_data();
        page.header.state = state;
        page
    }

    type Read = Pin<Box<dyn Future<Output = Result<commands::PackagePageData, String>>>>;
    type Lookup = Pin<Box<dyn Future<Output = Result<Option<String>, String>>>>;

    thread_local! {
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static LOOKUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn no_remote(_: String) -> Read {
        READS.set(READS.get() + 1);
        Box::pin(async { Ok(data(PackageState::NoRemote)) })
    }

    /// No remote first, then one, as choosing a bucket leaves it.
    fn then_linked(address: String) -> Read {
        if READS.get() == 0 {
            no_remote(address)
        } else {
            READS.set(READS.get() + 1);
            Box::pin(async { Ok(data(PackageState::Latest)) })
        }
    }

    fn diverged(_: String) -> Read {
        READS.set(READS.get() + 1);
        let mut d = data(PackageState::Diverged);
        d.context.resolve = Some(commands::ResolveData::Compared {
            published_message: Some("Theirs".to_string()),
            differing: vec!["plate/a.csv".to_string()],
            unpublished: 1,
            uncommitted: 0,
        });
        Box::pin(async move { Ok(d) })
    }

    fn answered(_: crate::routes::RevisionMismatch, _: String) -> Lookup {
        LOOKUPS.set(LOOKUPS.get() + 1);
        Box::pin(async { Ok(Some("Re-run plate 7".to_string())) })
    }

    async fn screen(address: &str, read: PageRead) -> web_sys::Element {
        unmount_earlier();
        READS.set(0);
        LOOKUPS.set(0);
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
                                        pull=super::super::tests::never_pulls
                                        resolving=ResolveCommands::app()
                                        revision_message=answered
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

    fn says(el: &web_sys::Element, text: &str) -> bool {
        el.text_content().unwrap_or_default().contains(text)
    }

    /// The line, verbatim, in a warning that waits for a pause, with nothing
    /// to press: it explains and offers nothing the header does not.
    #[wasm_bindgen_test]
    async fn the_band_explains_a_package_with_no_remote() {
        let el = screen(&format!("{PLAIN}&localOnly=1"), no_remote).await;

        let band = element_saying(&el, LINE)
            .closest("[role=status]")
            .unwrap()
            .expect("the band");
        assert!(
            band.query_selector("button").unwrap().is_none(),
            "markup was {}",
            band.inner_html()
        );
    }

    /// A warning, not a success or a failure: the page shows the reader's
    /// own copy, and nothing went wrong. Stylance hashes the class, so the
    /// test reads its stem.
    #[wasm_bindgen_test]
    async fn the_band_is_a_warning() {
        let el = screen(&format!("{PLAIN}&localOnly=1"), no_remote).await;

        let band = element_saying(&el, LINE)
            .closest("[role=status]")
            .unwrap()
            .expect("the band");
        let class = band.get_attribute("class").unwrap_or_default();
        assert!(
            class.split_whitespace().any(|c| c.starts_with("warning-")),
            "class was {class}"
        );
    }

    #[wasm_bindgen_test]
    async fn no_flag_no_band() {
        let el = screen(PLAIN, no_remote).await;

        element_saying(&el, "Revisions you have (1)");
        assert!(!says(&el, LINE), "markup was {}", el.inner_html());
    }

    /// Choosing a bucket re-reads the page with the flag still in the address;
    /// the reason the band gives is gone, and the band with it.
    #[wasm_bindgen_test]
    async fn the_band_goes_once_the_package_has_a_remote() {
        let el = screen(&format!("{PLAIN}&localOnly=1"), then_linked).await;
        element_saying(&el, LINE);

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.get(), 2, "re-read");
        assert!(!says(&el, LINE), "markup was {}", el.inner_html());
        assert!(search().contains("localOnly=1"), "address left alone");
    }

    /// An address that says both is read as local-only: with no remote there
    /// is no requested revision to compare, so no mismatch band and no lookup.
    #[wasm_bindgen_test]
    async fn local_only_wins_over_a_mismatch() {
        let el = screen(
            &format!("{PLAIN}&mismatch=c41d8f02&mrbucket=quilt-lab&localOnly=1"),
            no_remote,
        )
        .await;

        element_saying(&el, LINE);
        assert!(
            !says(&el, "Requested version"),
            "markup was {}",
            el.inner_html()
        );
        assert_eq!(LOOKUPS.get(), 0, "nothing to look up");
    }

    /// Entering the mode and leaving it keep the flag, as they keep a
    /// mismatch: a mode entered on the page must not end the band.
    #[wasm_bindgen_test]
    async fn entering_and_leaving_resolve_keep_local_only() {
        let plain = format!("{PLAIN}&localOnly=1");
        let el = screen(&plain, diverged).await;

        button_saying(&el, "Resolve").click();
        sleep_ms(50).await;
        assert_eq!(
            search(),
            "?namespace=team%2Fdataset&filter=unmodified&resolve=1&localOnly=1"
        );

        let back = el
            .query_selector(&format!("a[href='{plain}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("the back link keeps it; markup was {}", el.inner_html()));
        back.unchecked_into::<web_sys::HtmlElement>().click();
        sleep_ms(50).await;
        assert_eq!(
            search(),
            "?namespace=team%2Fdataset&filter=unmodified&localOnly=1"
        );
    }

    /// A `resolve=1` the package cannot honour is replaced, and the
    /// replacement keeps the flag and the band.
    #[wasm_bindgen_test]
    async fn a_normalised_address_keeps_local_only() {
        let el = screen(&format!("{PLAIN}&resolve=1&localOnly=1"), no_remote).await;

        assert_eq!(
            search(),
            "?namespace=team%2Fdataset&filter=unmodified&localOnly=1"
        );
        element_saying(&el, LINE);
    }
}

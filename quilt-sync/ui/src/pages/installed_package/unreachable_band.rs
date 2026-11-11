//! The band for a package whose host can't be reached.
//!
//! The header's chip names the condition and offers *Try again*; the band says
//! what it means for the page: what is shown is the last known state, and the
//! commands that need the host wait for it.

use leptos::prelude::*;

use quilt_ui_kit::{Banner, BannerVariant, PackageState};

/// The band itself, drawn after the outcome and the pause.
///
/// Warning, and no dismiss: it reports no failure of the reader's, and it
/// stands while the state does. The next read that reaches the host drops it.
pub(super) fn unreachable_band(state: &PackageState) -> AnyView {
    let PackageState::Unreachable { host } = state else {
        return ().into_any();
    };
    let host = host.as_deref().unwrap_or("the S3 bucket");
    view! {
        <Banner variant=BannerVariant::Warning>
            {format!(
                "Showing what this computer last knew about this package. \
                 Publish and Get latest wait until {host} can be reached."
            )}
        </Banner>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::super::{PackageScreen, PageRead, ResolveCommands};
    use super::*;
    use crate::commands;
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use quilt_ui_kit::test_support::{
        button_saying, element_saying, mount, sleep_ms, unmount_earlier,
    };
    use wasm_bindgen_test::*;

    const PLAIN: &str = "/installed-package?namespace=team%2Fdataset";
    const LINE: &str = "Showing what this computer last knew about this package. \
                        Publish and Get latest wait until quilt.test can be reached.";

    type Read = Pin<Box<dyn Future<Output = Result<commands::PackagePageData, String>>>>;

    thread_local! {
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn data(state: PackageState) -> commands::PackagePageData {
        let mut page = super::super::tests::page_data();
        page.header.state = state;
        page
    }

    fn unreachable(_: String) -> Read {
        READS.set(READS.get() + 1);
        Box::pin(async {
            Ok(data(PackageState::Unreachable {
                host: Some("quilt.test".to_string()),
            }))
        })
    }

    /// Unreachable first, then reached, as a host that comes back leaves it.
    fn then_reached(address: String) -> Read {
        if READS.get() == 0 {
            unreachable(address)
        } else {
            READS.set(READS.get() + 1);
            Box::pin(async { Ok(data(PackageState::Latest)) })
        }
    }

    /// Unreachable every time; reads after the first take a while to answer.
    fn slowly_unreachable(address: String) -> Read {
        let first = READS.get() == 0;
        let read = unreachable(address);
        Box::pin(async move {
            if !first {
                sleep_ms(300).await;
            }
            read.await
        })
    }

    fn never_asked(
        _: crate::routes::RevisionMismatch,
        _: String,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, String>>>> {
        Box::pin(async { Ok(None) })
    }

    async fn screen(read: PageRead) -> web_sys::Element {
        unmount_earlier();
        READS.set(0);
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(PLAIN))
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
                                        revision_message=never_asked
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

    fn says(el: &web_sys::Element, text: &str) -> bool {
        el.text_content().unwrap_or_default().contains(text)
    }

    /// The page draws, not its failure: the header names the host, and the
    /// band says what the page shows and what waits.
    #[wasm_bindgen_test]
    async fn an_unreachable_host_draws_the_page_and_the_band() {
        let el = screen(unreachable).await;

        assert!(says(&el, "Can't reach quilt.test"), "{}", el.inner_html());
        assert!(!says(&el, "Could not load"), "{}", el.inner_html());
        let band = element_saying(&el, LINE)
            .closest("[role=status]")
            .unwrap()
            .expect("the band");
        assert!(
            band.query_selector("button").unwrap().is_none(),
            "the header offers Try again, the band only explains: {}",
            band.inner_html()
        );
    }

    /// *Try again* reads the page again, and a read that reaches the host
    /// drops the band.
    #[wasm_bindgen_test]
    async fn try_again_reads_again_and_a_reached_host_drops_the_band() {
        let el = screen(then_reached).await;
        assert_eq!(READS.get(), 1);

        button_saying(&el, "Try again").click();
        sleep_ms(50).await;

        assert_eq!(READS.get(), 2);
        assert!(!says(&el, LINE), "{}", el.inner_html());
    }

    /// *Try again* spins and stays disabled while its read is out, as
    /// *Refresh* does, and comes back when the read answers.
    #[wasm_bindgen_test]
    async fn try_again_is_busy_until_its_read_answers() {
        let el = screen(slowly_unreachable).await;

        button_saying(&el, "Try again").click();
        sleep_ms(50).await;
        let button = button_saying(&el, "Try again");
        assert!(button.disabled(), "{}", button.outer_html());
        let class = button.get_attribute("class").unwrap_or_default();
        assert!(
            class.split_whitespace().any(|c| c.starts_with("loading-")),
            "class was {class}"
        );

        sleep_ms(400).await;
        assert_eq!(READS.get(), 2);
        assert!(!button_saying(&el, "Try again").disabled());
    }
}

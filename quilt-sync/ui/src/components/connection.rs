//! What the page shows of its connection to the engine, and what the engine
//! asks of every page.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::engine;
use crate::engine::Connection;
use crate::kit::{Banner, BannerVariant, Blankslate};

/// A bar while the page is reconnecting, and one that stays once the engine
/// refused it.
#[component]
pub fn ConnectionBanner() -> impl IntoView {
    let connection = engine::connection();
    move || match connection.get() {
        Connection::Rejected(message) => {
            Some(view! { <Banner variant=BannerVariant::Critical>{message}</Banner> }.into_any())
        }
        Connection::Lost => Some(
            view! {
                <Banner variant=BannerVariant::Warning>
                    "Lost the connection to QuiltSync. Reconnecting…"
                </Banner>
            }
            .into_any(),
        ),
        Connection::Connecting | Connection::Open(_) => None,
    }
}

/// The page opened without its link: say how to get one, rather than fail on
/// every section.
#[component]
pub fn NoLink() -> impl IntoView {
    view! {
        <Blankslate
            heading="Open QuiltSync from its link"
            description="This page needs the link QuiltSync gives it. Open it from the tray menu: Open in browser."
        />
    }
}

/// Go where the engine sends every page: after a login, or a deep link.
#[component]
pub fn FollowEngine() -> impl IntoView {
    let navigate = use_navigate();
    let listener = engine::listen::<String>(crate::commands::NAVIGATE_EVENT, move |page| {
        navigate(&page, NavigateOptions::default());
    });
    on_cleanup(move || drop(listener));
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;
    use crate::engine::fake::{FakeEngine, ask};
    use crate::test_support::{mount, sleep_ms};

    fn alerts(root: &web_sys::Element) -> u32 {
        root.query_selector_all("[role=alert]").unwrap().length()
    }

    /// A refused token stops the retries, fails what waits, and says so in
    /// one bar.
    #[wasm_bindgen_test]
    async fn a_refused_token_stops_retrying_with_one_banner() {
        let engine = FakeEngine::install();
        engine.refuse_token();
        let root = mount(ConnectionBanner);
        let asked = ask("get_toasts");
        sleep_ms(0).await;
        engine.close();
        sleep_ms(0).await;

        assert!(
            asked.borrow().clone().is_some_and(|answer| answer.is_err()),
            "what waited failed"
        );
        assert_eq!(engine.sockets_opened(), 1, "no retry after a refusal");
        assert_eq!(alerts(&root), 1);
        assert!(
            engine::invoke_unit::<()>("get_toasts")
                .await
                .is_err_and(|err| err.contains("new link")),
            "a later request fails at once and says why"
        );
        sleep_ms(0).await;
        assert_eq!(engine.sockets_opened(), 1);
        assert_eq!(alerts(&root), 1);
    }

    /// While the page reconnects it says so, and stops saying so once back.
    #[wasm_bindgen_test]
    async fn a_lost_connection_shows_until_the_engine_is_back() {
        let engine = FakeEngine::install();
        let root = mount(ConnectionBanner);
        let _listener = engine::listen::<serde_json::Value>("toast", |_| ());
        engine.hello();
        engine.close();
        sleep_ms(0).await;
        assert!(
            root.text_content()
                .unwrap_or_default()
                .contains("Reconnecting")
        );

        engine.hello();
        sleep_ms(0).await;
        assert_eq!(root.text_content().unwrap_or_default().trim(), "");
    }
}

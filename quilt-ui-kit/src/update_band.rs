//! A newer release of the app, as a band under the appbar.
//!
//! An update is state, true until it is installed, so it is a [`Banner`] in
//! the page's flow rather than an inbox entry. It keeps the checker's answers:
//! install, and dismiss, which snoozes the band; the caller owns the snooze.
//! The words are the checker's too: only how it draws changes.
//!
//! Attention, by the kit's What Happened Rule: a newer release is an ordinary
//! step outstanding, reached by nothing going wrong. A failed install is
//! Critical.

use leptos::prelude::*;

use super::Banner;
use super::BannerVariant;
use super::Button;
use super::Spinner;

/// What the checker knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    /// A newer version, offered.
    Available { version: String },
    /// Downloading and installing; the app restarts when it is done.
    Installing,
    /// The install failed and may be tried again.
    Failed { version: String, error: String },
}

#[component]
pub fn UpdateBand(
    #[prop(into)] state: Signal<UpdateState>,
    #[prop(into)] on_install: Callback<()>,
    /// Hides the band for a while. Not offered while installing, which cannot be
    /// stopped from here.
    #[prop(into)]
    on_snooze: Callback<()>,
) -> impl IntoView {
    move || match state.get() {
        UpdateState::Available { version } => view! {
            <Banner
                variant=BannerVariant::Warning
                on_dismiss=move |_| on_snooze.run(())
                action=view! {
                    <Button on_click=move |_| on_install.run(())>"Download & Install"</Button>
                }
                    .into_any()
            >
                "Update available: "
                {version}
            </Banner>
        }
        .into_any(),
        UpdateState::Installing => view! {
            <Banner variant=BannerVariant::Warning>
                <Spinner />
                " Downloading and installing update\u{2026}"
            </Banner>
        }
        .into_any(),
        UpdateState::Failed { version, error } => view! {
            <Banner
                variant=BannerVariant::Critical
                on_dismiss=move |_| on_snooze.run(())
                action=view! { <Button on_click=move |_| on_install.run(())>"Retry"</Button> }
                    .into_any()
            >
                "Update "
                {version}
                " failed: "
                {error}
            </Banner>
        }
        .into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::button_saying;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn band(state: UpdateState) -> (web_sys::Element, RwSignal<u32>, RwSignal<u32>) {
        let installs = RwSignal::new(0);
        let snoozes = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <UpdateBand
                    state=state
                    on_install=move |()| installs.update(|n| *n += 1)
                    on_snooze=move |()| snoozes.update(|n| *n += 1)
                />
            }
        });
        (el, installs, snoozes)
    }

    #[wasm_bindgen_test]
    fn an_update_offers_install_and_dismiss() {
        let (el, installs, snoozes) = band(UpdateState::Available {
            version: "0.42.0".into(),
        });
        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("0.42.0"), "{text}");
        button_saying(&el, "Download & Install").click();
        assert_eq!(installs.get_untracked(), 1);
        el.query_selector("button[aria-label='Dismiss']")
            .unwrap()
            .expect("a dismiss")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        assert_eq!(snoozes.get_untracked(), 1);
    }

    #[wasm_bindgen_test]
    fn installing_offers_nothing() {
        let (el, _, _) = band(UpdateState::Installing);
        assert!(el.query_selector("button").unwrap().is_none());
        assert!(
            el.query_selector("[role=status]").unwrap().is_some(),
            "and waits politely"
        );
    }

    #[wasm_bindgen_test]
    fn a_failure_is_an_alert_with_retry() {
        let (el, installs, _) = band(UpdateState::Failed {
            version: "0.42.0".into(),
            error: "the download was cut short".into(),
        });
        assert!(el.query_selector("[role=alert]").unwrap().is_some());
        button_saying(&el, "Retry").click();
        assert_eq!(installs.get_untracked(), 1);
    }
}

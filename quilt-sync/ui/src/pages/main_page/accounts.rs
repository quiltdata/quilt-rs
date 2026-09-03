//! The Accounts card: one row per host, its session and its role.
//!
//! # A settling row is rebuilt, not poked
//!
//! [`HostRow`] takes `roles` and `signed_out` as plain values and picks its
//! sub-line once, at build time, with `role.get_untracked()`. Writing a settled
//! role into the `role` signal would therefore leave a row reading
//! "Role unavailable" forever, and a row that gains a second role would never
//! gain a switcher. So [`AccountRow`] holds the payload in a signal and renders
//! `HostRow` inside a reactive closure: settling replaces the whole component.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::commands::AccountHostData;
use crate::commands::MainPageAccountsData;
use crate::kit::Card;
use crate::kit::HostRow;

/// Where the [Sign in] button goes. `pages/login.rs` reads both parameters from
/// the query string; `back` is what returns the user to this page afterwards.
fn sign_in_href(host: &str) -> String {
    format!("/login?host={host}&back=/main")
}

/// The card, on one payload. Split from [`AccountsCard`] so it can be tested
/// without a Tauri host, the same way [`AutosyncBody`](super::autosync) is.
///
/// The rows are direct children of the card body — no wrapper element, so the
/// card's own `.body > * + *` rule keeps spacing them.
#[component]
fn AccountsBody(data: MainPageAccountsData) -> impl IntoView {
    view! {
        <Card title="Accounts">
            {data
                .hosts
                .into_iter()
                .map(|host| view! { <AccountRow host=host /> })
                .collect_view()}
        </Card>
    }
}

/// One host, with its own heavy-phase settle — the shape `PackageListRow` uses:
/// one `spawn_local` per row so a row settles where it stands, and a cancel flag
/// set in `on_cleanup` so a row unmounted mid-flight never writes.
#[component]
fn AccountRow(host: AccountHostData) -> impl IntoView {
    let navigate = use_navigate();
    let row = RwSignal::new(host);

    // Only a signed-in host whose role is not yet known has anything to ask. A
    // settled row — and every signed-out row, which has no session to ask about —
    // issues no call at all; that split is the whole point of the light phase.
    if row.with_untracked(|host| host.provisional) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_flag = cancelled.clone();
        on_cleanup(move || cancelled.store(true, Ordering::Relaxed));

        let name = row.with_untracked(|host| host.host.clone());
        leptos::task::spawn_local(async move {
            let result = commands::refresh_main_page_account(name).await;
            if cancelled_flag.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(settled) => row.set(settled),
                Err(err) => {
                    // The row keeps what the light phase gave it, which is honest:
                    // nothing confirmed the role. Logged, never rendered.
                    web_sys::console::error_1(
                        &format!("refresh_main_page_account failed: {err}").into(),
                    );
                }
            }
        });
    }

    // Rebuilt whole on settle — see the module docs for why nothing here may be a
    // reactive prop threaded into `HostRow`.
    move || {
        let host = row.get();
        let target = sign_in_href(&host.host);
        let navigate = navigate.clone();
        // `HostRow` wants the role as a signal because its switcher writes to it.
        // Empty string is its own convention for "cannot be named" — see R5.
        let role = RwSignal::new(host.current_role.unwrap_or_default());

        view! {
            <HostRow
                host=host.host
                role=role
                roles=host.roles
                signed_out=!host.signed_in
                on_sign_in=move |_| navigate(&target, NavigateOptions::default())
            />
        }
    }
}

/// The card and its payload.
///
/// Same shape as [`AutosyncCard`](super::autosync::AutosyncCard): `Transition`
/// rather than `Suspense` (§6), the page's Refresh feeding the same resource as
/// the card's own reasons, and a body rebuilt from plain values. A failed fetch
/// renders nothing and logs — asserting anything about a user's sessions on the
/// strength of a failed read would be a manufactured state.
#[component]
pub fn AccountsCard(
    /// The page's own reload trigger, so the appbar's Refresh refetches this card
    /// as well as the package rows.
    refresh: Trigger,
) -> impl IntoView {
    let reload = Trigger::new();
    let accounts = LocalResource::new(move || {
        reload.track();
        refresh.track();
        commands::get_main_page_accounts()
    });

    view! {
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                match accounts.await {
                    Ok(data) => view! { <AccountsBody data=data /> }.into_any(),
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("get_main_page_accounts failed: {err}").into(),
                        );
                        ().into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AccountHostData;
    use crate::commands::MainPageAccountsData;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// Inside a `Router`, because [`AccountRow`] asks for `use_navigate` — the
    /// [Sign in] affordance is a navigation, and `use_navigate` panics outside a
    /// `Router`. Same shape `main_page.rs`'s own `renders_a_packages_card` uses.
    fn mount_body(data: MainPageAccountsData) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <AccountsBody data=data />
                </leptos_router::components::Router>
            }
        })
    }

    /// Every fixture is settled (`provisional: false`). There is no Tauri host
    /// under `wasm-bindgen-test`, so a provisional row would spawn an invoke that
    /// can only fail — noise in the log, and a settle no assertion here wants.
    fn two_hosts() -> MainPageAccountsData {
        MainPageAccountsData {
            hosts: vec![
                AccountHostData {
                    host: "open.quiltdata.com".to_string(),
                    signed_in: true,
                    current_role: Some("ReadWriteQuiltBucket".to_string()),
                    roles: vec!["ReadWriteQuiltBucket".to_string()],
                    provisional: false,
                },
                AccountHostData {
                    host: "solo.registry.io".to_string(),
                    signed_in: false,
                    current_role: None,
                    roles: Vec::new(),
                    provisional: false,
                },
            ],
        }
    }

    fn one_switchable_one_not() -> MainPageAccountsData {
        MainPageAccountsData {
            hosts: vec![
                AccountHostData {
                    host: "many.quiltdata.com".to_string(),
                    signed_in: true,
                    current_role: Some("ReadOnly".to_string()),
                    roles: vec!["ReadOnly".to_string(), "ReadWriteQuiltBucket".to_string()],
                    provisional: false,
                },
                AccountHostData {
                    host: "solo.registry.io".to_string(),
                    signed_in: true,
                    current_role: Some("ReadOnly".to_string()),
                    roles: vec!["ReadOnly".to_string()],
                    provisional: false,
                },
            ],
        }
    }

    fn nameless_role() -> MainPageAccountsData {
        MainPageAccountsData {
            hosts: vec![AccountHostData {
                host: "quiet.quiltdata.com".to_string(),
                signed_in: true,
                current_role: None,
                roles: Vec::new(),
                provisional: false,
            }],
        }
    }

    #[wasm_bindgen_test]
    fn a_signed_out_host_offers_a_way_back_in() {
        // The one affordance a signed-out row has. `HostRow` renders "Signed out"
        // plus a [Sign in] button and no role line at all.
        let el = mount_body(two_hosts());
        let text = el.text_content().unwrap();
        assert!(text.contains("Signed out"), "got: {text}");
        assert!(text.contains("Sign in"), "got: {text}");
    }

    #[wasm_bindgen_test]
    fn a_switchable_host_draws_a_switcher_and_a_single_role_does_not() {
        // `HostRow` gates the switcher on `roles.len() > 1` — a select with one
        // option is a dead control. One row here holds two roles, the other one.
        let el = mount_body(one_switchable_one_not());
        assert_eq!(
            el.query_selector_all("select").unwrap().length(),
            1,
            "exactly the host with a choice"
        );
    }

    #[wasm_bindgen_test]
    fn a_nameless_role_says_so_rather_than_leaving_a_blank() {
        // R5 end to end: `currentRole: null` on a signed-in host is "Role
        // unavailable", never an empty cell and never "Signed out".
        let el = mount_body(nameless_role());
        let text = el.text_content().unwrap();
        assert!(text.contains("Role unavailable"), "got: {text}");
        assert!(
            !text.contains("Signed out"),
            "a failed query is not a logout: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn every_host_gets_exactly_one_row() {
        let el = mount_body(two_hosts());
        assert_eq!(
            el.query_selector_all("button").unwrap().length(),
            1,
            "one [Sign in], for the one signed-out host"
        );
        assert!(el.text_content().unwrap().contains("open.quiltdata.com"));
        assert!(el.text_content().unwrap().contains("solo.registry.io"));
    }

    #[wasm_bindgen_test]
    fn the_sign_in_link_carries_the_host_and_the_way_back() {
        // R6. `pages/login.rs` reads `host` and `back` from the query string, so a
        // link missing either lands the user on a login page that cannot come back.
        assert_eq!(
            sign_in_href("custom.registry.io"),
            "/login?host=custom.registry.io&back=/main"
        );
    }
}

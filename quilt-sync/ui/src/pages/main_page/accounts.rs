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
///
/// `pub(super)`, not private: the queue's own `[Sign in]` (§4.3) reuses this
/// rather than writing a second copy of the format string.
pub(super) fn sign_in_href(host: &str) -> String {
    format!("/login?host={host}&back=/main")
}

/// The card, on one payload. Split from [`AccountsCard`] so it can be tested
/// without a Tauri host, the same way [`AutosyncBody`](super::autosync) is.
///
/// The rows are direct children of the card body — no wrapper element, so the
/// card's own `.body > * + *` rule keeps spacing them.
#[component]
fn AccountsBody(
    data: MainPageAccountsData,
    /// The **page's** reload trigger, notified after a role switch. A switch moves
    /// more than this card — see [`AccountRow`].
    refresh: Trigger,
) -> impl IntoView {
    view! {
        <Card title="Accounts">
            {data
                .hosts
                .into_iter()
                .map(|host| view! { <AccountRow host=host refresh=refresh /> })
                .collect_view()}
        </Card>
    }
}

/// One host, with its own heavy-phase settle — the shape `PackageListRow` uses:
/// one `spawn_local` per row so a row settles where it stands, and a cancel flag
/// set in `on_cleanup` so a row unmounted mid-flight never writes.
#[component]
fn AccountRow(
    host: AccountHostData,
    /// The page's own trigger, notified after a role switch, on success and on
    /// failure alike — see the effect below.
    refresh: Trigger,
) -> impl IntoView {
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
        let switch_host = host.host.clone();
        // `HostRow` wants the role as a signal because its switcher writes to it.
        // Empty string is its own convention for "cannot be named" — see R5. It is
        // `provisional` that tells the row apart from a settled failure, so the two
        // must be passed together.
        let current = host.current_role.unwrap_or_default();
        // Seeded from the payload rather than from the signal, and re-seeded with
        // it on every rebuild, so a row handed a role by a refetch cannot write
        // that same role straight back.
        let settled = StoredValue::new(current.clone());
        let role = RwSignal::new(current);

        // Owned by the render effect running this closure: its `with_cleanup`
        // drops the arena node holding this effect before each re-run, so a
        // rebuild replaces the effect instead of stacking a second one.
        Effect::new(move |_| {
            let chosen = role.get();
            if chosen.is_empty() || chosen == settled.get_value() {
                return;
            }
            settled.set_value(chosen.clone());
            let host_name = switch_host.clone();
            leptos::task::spawn_local(async move {
                if let Err(err) = commands::switch_role(host_name, chosen).await {
                    web_sys::console::error_1(&format!("switch_role failed: {err}").into());
                }
                // The page's trigger, not a card-private one: a switch expires the
                // host's credentials, drops the cached clients holding them and
                // releases role-denied pauses, so it moves the package rows' access
                // marks and any role-denied pause as surely as it moves this card.
                // Notified on failure too — the refetch is what puts the true role
                // back on screen.
                refresh.notify();
            });
        });

        view! {
            <HostRow
                host=host.host
                role=role
                roles=host.roles
                signed_out=!host.signed_in
                provisional=host.provisional
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
    /// The page's own reload trigger. It feeds this card, so the appbar's Refresh
    /// refetches it alongside the package rows — and a role switch notifies the
    /// same trigger, so the switch refetches the page rather than only itself.
    refresh: Trigger,
) -> impl IntoView {
    let accounts = LocalResource::new(move || {
        refresh.track();
        commands::get_main_page_accounts()
    });

    view! {
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                match accounts.await {
                    Ok(data) => view! { <AccountsBody data=data refresh=refresh /> }.into_any(),
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

    /// A promise-backed sleep, the same four lines over `set_timeout` that
    /// [`autosync`](super::super::autosync)'s tests use.
    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    /// Pick a role the way a user does: set the value, then fire `change` — the
    /// event `Select` listens for. The value must be one of the rendered options;
    /// a `<select>` handed anything else reports the empty string instead.
    fn select_role(select: &web_sys::Element, role: &str) {
        let select: &web_sys::HtmlSelectElement = select.unchecked_ref();
        select.set_value(role);
        select
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
    }

    /// Inside a `Router`, because [`AccountRow`] asks for `use_navigate` — the
    /// [Sign in] affordance is a navigation, and `use_navigate` panics outside a
    /// `Router`. Same shape `main_page.rs`'s own `renders_a_packages_card` uses.
    fn mount_body(data: MainPageAccountsData) -> web_sys::Element {
        mount_body_refreshing(data, Trigger::new())
    }

    /// [`mount_body`] with the caller's own trigger, for the one test that counts
    /// what the card asks for. The trigger is the **page's** — the one
    /// `AccountsCard` also feeds its resource from — so counting it counts the
    /// refetch of the whole page, which is what a role switch owes.
    fn mount_body_refreshing(data: MainPageAccountsData, refresh: Trigger) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <AccountsBody data=data refresh=refresh />
                </leptos_router::components::Router>
            }
        })
    }

    /// Every fixture but [`waiting_for_a_role`] is settled (`provisional: false`).
    /// There is no Tauri host under `wasm-bindgen-test`, so a provisional row
    /// spawns an invoke that can only fail — noise in the log, and a settle no
    /// assertion here wants.
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

    /// The one provisional fixture: a signed-in host whose role nobody has asked
    /// for yet. Identical to [`nameless_role`] in every field but `provisional`,
    /// which is the point — the two states are told apart by that flag alone.
    ///
    /// It does spawn a `refresh_main_page_account`, and that is harmless here:
    /// `tauri_invoke_catching` is `catch`-bound, so a missing `window.__TAURI__`
    /// comes back as an `Err` rather than trapping the module (`tauri.rs:15-23`).
    /// The row therefore never settles, the assertions below cannot race it, and
    /// the whole cost is one `console.error` line.
    fn waiting_for_a_role() -> MainPageAccountsData {
        MainPageAccountsData {
            hosts: vec![AccountHostData {
                host: "quiet.quiltdata.com".to_string(),
                signed_in: true,
                current_role: None,
                roles: Vec::new(),
                provisional: true,
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
    fn a_row_waiting_for_its_role_does_not_claim_the_query_failed() {
        // "Role unavailable" is the *settled* failure: we asked and could not tell.
        // Saying it before anyone has asked makes a cold cache — which every first
        // paint is, since the roster load invalidates the role cache — look exactly
        // like an offline host.
        //
        // Both halves run here on purpose. The two fixtures differ by `provisional`
        // and nothing else, so a test asserting against only one of them would pass
        // on a component that ignored the flag entirely.
        let waiting = mount_body(waiting_for_a_role());
        let text = waiting.text_content().unwrap();
        assert!(
            !text.contains("Role unavailable"),
            "nobody has asked yet: {text}"
        );
        assert!(text.contains("Checking role"), "and it says so: {text}");
        assert!(
            !text.contains("Signed out"),
            "the session is a fact from disk: {text}"
        );
        assert!(
            waiting
                .query_selector("[class*='provisional']")
                .unwrap()
                .is_some(),
            "and it is drawn as not-yet-final"
        );

        let settled = mount_body(nameless_role());
        let text = settled.text_content().unwrap();
        assert!(
            text.contains("Role unavailable"),
            "we asked and could not tell: {text}"
        );
        assert!(
            !text.contains("Checking role"),
            "nothing is in flight: {text}"
        );
        assert!(
            settled
                .query_selector("[class*='provisional']")
                .unwrap()
                .is_none(),
            "a settled failure is final"
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
    async fn choosing_a_role_asks_the_backend_again() {
        // The write and the refetch are one gesture: switching a role expires
        // credentials, drops cached clients and clears role-denied pauses, so the
        // payload after a switch differs by more than the name the user picked —
        // and by more than this card holds, which is why the trigger counted here
        // is the page's own and not one private to the card.
        //
        // There is no Tauri host here, so `switch_role` can only fail — which is
        // exactly why the trigger must fire on the error path too.
        let fired = RwSignal::new(0);
        let refresh = Trigger::new();
        Effect::new(move |_| {
            refresh.track();
            fired.update(|n| *n += 1);
        });
        let el = mount_body_refreshing(one_switchable_one_not(), refresh);

        // Pin the baseline: the effect's own first run has happened and nothing
        // has been switched yet. Both waits are absolute, neither derived from
        // what is under test.
        sleep_ms(50).await;
        assert_eq!(
            fired.get_untracked(),
            1,
            "the effect's own first run, before any switch"
        );

        // "ReadWriteQuiltBucket" is the role the fixture is not on, so the switch
        // is a real change rather than a value the row already held.
        let select = el.query_selector("select").unwrap().unwrap();
        select_role(&select, "ReadWriteQuiltBucket");
        sleep_ms(200).await;

        assert_eq!(fired.get_untracked(), 2, "one refetch, and exactly one");
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

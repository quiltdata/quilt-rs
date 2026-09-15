//! One host in the Accounts card.
//!
//! Roles are per-host (`switch_role(host, role_name)`); there is no global role,
//! so this row is where a role is read and changed. The switcher appears only
//! where there is a choice, matching `role_switch_host`, which is `Some` exactly
//! when the user holds more than one role at that host.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

use crate::kit::Button;
use crate::kit::Naming;
use crate::kit::Select;

stylance::import_crate_style!(style, "src/kit/host_row.module.scss");

#[component]
pub fn HostRow(
    #[prop(into)] host: String,
    /// The active role. Empty means the role query failed — the session is fine,
    /// it simply cannot be named, so the row says so rather than showing a blank.
    role: RwSignal<String>,
    /// Every role held at this host. One or none means no switcher: a select with
    /// a single option is a dead control.
    #[prop(optional)]
    roles: Vec<String>,
    #[prop(optional)] signed_out: bool,
    /// The role has not been asked for yet — the light phase named the session,
    /// the heavy one has not answered. Draws the sub-line with a dashed rule and
    /// settles to solid when the real role arrives.
    ///
    /// **Dashed rather than dimmed**, the same choice [`StateLabel`](super::StateLabel)
    /// makes and for the same reason: this sub-line is already the muted step, and any
    /// opacity low enough to read as provisional takes it under AA, while a dashed rule
    /// costs no contrast at all. A row waiting for its role is still saying something
    /// true, so making it harder to read in order to say "not final" trades the wrong
    /// thing.
    ///
    /// Read once, at build time, like `role` and `roles` — a row that settles is
    /// rebuilt, never poked. See `pages/main_page/accounts.rs`'s module doc.
    #[prop(optional, into)]
    provisional: MaybeProp<bool>,
    on_sign_in: impl Fn(MouseEvent) + 'static,
) -> impl IntoView {
    let switchable = roles.len() > 1;
    let waiting = provisional.get_untracked().unwrap_or(false);

    // Exactly one sub-line, always present. Signed out outranks the role, because
    // a role means nothing without a session; a role not yet asked for outranks
    // every reading of the role itself, because there is nothing yet to read.
    let sub = if signed_out {
        Some(view! { <span class=style::warning>"Signed out"</span> }.into_any())
    } else if waiting {
        // The row is rebuilt when the role lands, so without a live region the
        // answer to `Checking role…` never reaches a screen reader.
        Some(view! { <span role="status">"Checking role\u{2026}"</span> }.into_any())
    } else if switchable {
        // The role is shown by the switcher itself, so repeating it here would
        // say the same thing twice.
        None
    } else if role.get_untracked().is_empty() {
        Some(view! { "Role unavailable" }.into_any())
    } else {
        Some(view! { {move || format!("Role: {}", role.get())} }.into_any())
    };

    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_host = host.clone();
    view! {
        <div class=style::root>
            <span class=style::text>
                <span class=style::host title=full_host>{host}</span>
                {sub
                    .map(|line| {
                        let class = if waiting {
                            format!("{} {}", style::sub, style::provisional)
                        } else {
                            style::sub.to_string()
                        };
                        view! { <span class=class>{line}</span> }
                    })}
            </span>
            <span class=style::trailing>
                {if signed_out {
                    // A signed-out host has no role to pick, so the only
                    // affordance is getting the session back.
                    view! { <Button on_click=on_sign_in>"Sign in"</Button> }.into_any()
                } else if switchable {
                    view! {
                        <Select naming=Naming::Prefix("Role".to_string()) options=roles selected=role />
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
            </span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `main_page.rs`'s pattern: mount a view into a fresh, attached `div` and
    /// hand back the element to query against.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// A catalog host is a domain, and a long one truncates.
    #[wasm_bindgen_test]
    fn the_truncating_host_carries_its_whole_value() {
        let el = mount(|| {
            view! {
                <HostRow
                    host="a-very-long-catalog-hostname.example.quiltdata.com"
                    role=RwSignal::new("analyst".to_string())
                    on_sign_in=|_| {}
                />
            }
        });
        let span = el
            .query_selector("[class*=host]")
            .unwrap()
            .expect("the host");
        assert_eq!(
            span.get_attribute("title").as_deref(),
            Some("a-very-long-catalog-hostname.example.quiltdata.com"),
        );
    }
    /// The row is rebuilt when the role lands, so the answer to `Checking role…`
    /// reaches a screen reader only through a live region.
    #[wasm_bindgen_test]
    fn the_role_being_checked_is_a_live_region() {
        let el = mount(|| {
            view! {
                <HostRow
                    host="open.quiltdata.com"
                    role=RwSignal::new(String::new())
                    provisional=true
                    on_sign_in=|_| {}
                />
            }
        });
        let status = el
            .query_selector("[role=status]")
            .unwrap()
            .expect("a live region while the role is unknown");
        assert!(status.text_content().unwrap().contains("Checking role"));
    }
}

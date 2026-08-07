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
    on_sign_in: impl Fn(MouseEvent) + 'static,
) -> impl IntoView {
    let switchable = roles.len() > 1;

    // Exactly one sub-line, always present. Signed out outranks the role, because
    // a role means nothing without a session.
    let sub = if signed_out {
        Some(view! { <span class=style::warning>"Signed out"</span> }.into_any())
    } else if switchable {
        // The role is shown by the switcher itself, so repeating it here would
        // say the same thing twice.
        None
    } else if role.get_untracked().is_empty() {
        Some(view! { "Role unavailable" }.into_any())
    } else {
        Some(view! { {move || format!("Role: {}", role.get())} }.into_any())
    };

    view! {
        <div class=style::root>
            <span class=style::text>
                <span class=style::host>{host}</span>
                {sub.map(|line| view! { <span class=style::sub>{line}</span> })}
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

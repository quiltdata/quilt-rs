//! The dialog a denial's remedy opens: pick another role, and the header
//! re-reads.
//!
//! A `Select` and not a list of buttons, because picking a role is picking a
//! VALUE — the kit's own split between `Select` and `ActionMenu`. The switch
//! is the dialog's submit.
//!
//! The options are the alternatives the payload carried, which already
//! exclude the refused role: a select whose current value is the thing that
//! failed offers a no-op as its default.
//!
//! The refused role is named in the dialog, in one sentence before the select,
//! and not in the header's chip: the chip is one state label read at a glance,
//! and the dialog is where the reader chooses what replaces the role.

use leptos::prelude::*;

use crate::commands;
use crate::kit::{FormControl, FormDialog, Naming, Select, Submit};

use super::{Wiring, holding};

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/role_dialog.module.scss"
);

/// The role switch, over the alternatives the payload offers. `refused` is the
/// role the denial named, `None` when the backend could not name one.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub(super) fn RoleDialog(
    open: RwSignal<bool>,
    switch: commands::RoleSwitch,
    refused: Option<String>,
    w: Wiring,
) -> impl IntoView {
    // A refusal is the dialog's banner, and nothing goes to the band; starting
    // the command only retracts what the band said last. `busy` is held for the
    // command and seals the dialog, because a re-read rebuilds this dialog and
    // its own seal goes with the old one.
    let Wiring {
        busy,
        outcome,
        reload,
        ..
    } = w;
    let commands::RoleSwitch { host, alternatives } = switch;
    let chosen = RwSignal::new(alternatives.first().cloned().unwrap_or_default());

    // Each opening starts from the first alternative, not from what was picked
    // in the last one and cancelled.
    let first = alternatives.first().cloned().unwrap_or_default();
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if is_open && was_open == Some(false) {
            chosen.set(first.clone());
        }
        is_open
    });

    // The backend clears this host's role-denied pauses on a switch, so the
    // re-read is all that is left to do.
    let submit = Submit::new("Switch", move || {
        let host = host.clone();
        async move {
            let role = chosen.get_untracked();
            holding(busy, outcome, commands::switch_role(host, role.clone()))
                .await
                .map_err(|err| format!("Could not switch to {role}: {err}"))?;
            reload.notify();
            Ok(())
        }
    });

    view! {
        <FormDialog open=open title="Switch role" submit=submit running=busy>
            <p class=style::refusal>{refusal(refused)}</p>
            <FormControl
                label="Role"
                control=move |id| {
                    view! {
                        <Select
                            naming=Naming::FormControl(id)
                            options=alternatives
                            selected=chosen
                        />
                    }
                        .into_any()
                }
            />
        </FormDialog>
    }
}

/// Why the dialog is open, for the sentence before its select. A denial that
/// could not name the role still has words, without inventing a name.
fn refusal(refused: Option<String>) -> AnyView {
    match refused {
        Some(role) => view! {
            "You're using " <strong>{role}</strong> ", which can't read this package."
        }
        .into_any(),
        None => "Your current role can't read this package.".into_any(),
    }
}

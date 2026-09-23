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

use leptos::prelude::*;

use crate::commands;
use crate::kit::{FormControl, FormDialog, Naming, Select, Submit};

use super::Wiring;

/// The role switch, over the alternatives the payload offers.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub(super) fn RoleDialog(
    open: RwSignal<bool>,
    switch: commands::RoleSwitch,
    w: Wiring,
) -> impl IntoView {
    // Only `reload`: `FormDialog` seals itself while its submit runs, and a
    // refusal is its banner's, inside the dialog — nothing goes to the band.
    let Wiring {
        busy: _,
        outcome: _,
        reload,
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
            commands::switch_role(host, chosen.get_untracked()).await?;
            reload.notify();
            Ok(())
        }
    });

    view! {
        <FormDialog open=open title="Switch role" submit=submit>
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

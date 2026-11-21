//! The local registry's name on the settings page, with the dataspace
//! engine on: the name packages made here are kept under, and its rename,
//! after which the old name still names it (25.10). Not drawn while the
//! engine is off.

use leptos::prelude::*;
use quilt_ui_kit::Button;
use quilt_ui_kit::Card;
use quilt_ui_kit::FormControl;
use quilt_ui_kit::TextInput;

use crate::commands;

#[component]
pub fn LocalRegistryCard() -> impl IntoView {
    let renamed = Trigger::new();
    let name = LocalResource::new(move || {
        renamed.track();
        commands::get_local_registry()
    });
    let typed = RwSignal::new(String::new());
    let said = RwSignal::new(None::<String>);
    let rename = move |_| {
        let wanted = typed.get_untracked();
        leptos::task::spawn_local(async move {
            let answer = commands::rename_local_registry(wanted).await;
            said.try_set(Some(answer.unwrap_or_else(|err| err)));
            renamed.notify();
        });
    };
    move || {
        let current = name.get().and_then(Result::ok).flatten()?;
        Some(view! {
            <Card title="Local registry">
                <FormControl
                    label="Name"
                    caption=format!(
                        "Packages made on this computer are kept as {current}. A rename keeps the old name working.",
                    )
                    control=move |id| {
                        view! { <TextInput id=id value=typed placeholder=current.clone() /> }
                            .into_any()
                    }
                />
                <Button on_click=rename>"Rename"</Button>
                {move || said.get().map(|words| view! { <p role="status">{words}</p> })}
            </Card>
        })
    }
}

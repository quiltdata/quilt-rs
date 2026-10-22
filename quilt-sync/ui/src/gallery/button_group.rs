//! `ButtonGroup` stories, in the package header's location group.

use std::time::Duration;

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ButtonGroup;
use crate::kit::GroupButton;
use crate::kit::icons;

/// How long a copy segment says `Copied`, as long as the page's header does.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The package header's group: the local folder, then the remote side when
/// there is a catalog page. `copied` stands in for the page's clipboard state.
pub fn location(remote: bool, copied: RwSignal<bool>) -> AnyView {
    group(remote, copied, false)
}

/// [`location`], with the page's lock held when `busy`.
fn group(remote: bool, copied: RwSignal<bool>, busy: bool) -> AnyView {
    let busy = Signal::stored(busy);
    let mut buttons = vec![
        GroupButton::labelled(icons::folder, "Open folder", Callback::new(|()| ()))
            .with_disabled(busy),
    ];
    if remote {
        let pending: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
        buttons.push(
            GroupButton::labelled(
                icons::link_external,
                "Open in catalog",
                Callback::new(|()| ()),
            )
            .with_disabled(busy),
        );
        buttons.push(
            GroupButton::icon(
                icons::copy,
                "Copy Quilt URI",
                Callback::new(move |()| {
                    copied.set(true);
                    // A repeat click starts the time again.
                    if let Some(Some(earlier)) = pending.try_update_value(Option::take) {
                        earlier.clear();
                    }
                    let later = set_timeout_with_handle(
                        move || {
                            copied.try_set(false);
                        },
                        COPIED_FOR,
                    )
                    .ok();
                    pending.try_set_value(later);
                }),
            )
            .with_done("Copied", copied.into()),
        );
    }
    view! { <ButtonGroup aria_label="Package location" buttons=buttons /> }.into_any()
}

/// A container narrower than the group's breakpoint.
fn narrow(children: AnyView) -> AnyView {
    view! { <div style="container-type:inline-size; width:360px; max-width:100%">{children}</div> }
        .into_any()
}

const NOTE: &str = "Joined segments for one subject: here a package's folder on the left and \
                    its catalog page and Quilt URI on the right. A segment with a label shows \
                    it while its container is wider than 800px, and opens no tooltip, since \
                    the words are already drawn. Narrower, every segment is its icon and its \
                    name moves to a tooltip: hover or Tab to one. The copy segment is always \
                    an icon; click it and its tooltip says Copied for a moment. Without a \
                    catalog page the remote side is dropped. While another command runs \
                    the two opening segments are disabled and the copy is not.";

#[component]
pub fn ButtonGroupStories() -> impl IntoView {
    view! {
        <Story title="ButtonGroup" note=NOTE>
            <Cell wide=true label="wide, with a catalog page">
                {location(true, RwSignal::new(false))}
            </Cell>
            <Cell wide=true label="wide, not published yet: no catalog page">
                {location(false, RwSignal::new(false))}
            </Cell>
            <Cell wide=true label="narrow: icons only, hover each for its name">
                {narrow(location(true, RwSignal::new(false)))}
            </Cell>
            <Cell wide=true label="narrow, not published yet">
                {narrow(location(false, RwSignal::new(false)))}
            </Cell>
            <Cell wide=true label="another command running: only the copy is live">
                {group(true, RwSignal::new(false), true)}
            </Cell>
            <Cell wide=true label="copied: hover the copy icon, it reads Copied">
                {location(true, RwSignal::new(true))}
            </Cell>
        </Story>
    }
}

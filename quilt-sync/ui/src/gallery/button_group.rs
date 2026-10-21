//! `ButtonGroup` stories, in the package header's location group.

use std::time::Duration;

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ButtonGroup;
use crate::kit::GroupButton;
use crate::kit::icons;

/// How long a copy segment says `Copied`. The page will own this timer.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The package header's group: the local folder, then the remote side when
/// there is a catalog page. `copied` stands in for the page's clipboard state.
pub fn location(remote: bool, copied: RwSignal<bool>) -> AnyView {
    let mut buttons = vec![GroupButton::labelled(
        icons::folder,
        "Open folder",
        Callback::new(|()| ()),
    )];
    if remote {
        buttons.push(GroupButton::labelled(
            icons::link_external,
            "Open in catalog",
            Callback::new(|()| ()),
        ));
        buttons.push(
            GroupButton::icon(
                icons::copy,
                "Copy Quilt URI",
                Callback::new(move |()| {
                    copied.set(true);
                    set_timeout(
                        move || {
                            copied.try_set(false);
                        },
                        COPIED_FOR,
                    );
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
                    catalog page the remote side is dropped.";

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
            <Cell wide=true label="copied: hover the copy icon, it reads Copied">
                {location(true, RwSignal::new(true))}
            </Cell>
        </Story>
    }
}

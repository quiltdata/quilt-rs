//! The v2 package page's header: trail, identity, one resolved state, one
//! action, `Open folder`, and everything else behind `[⋯]`.
//!
//! # Page-local, not a kit piece
//!
//! One page draws this, and the pieces it is built from are the kit's. A
//! component would be a wrapper with one caller. The gallery scene at
//! `gallery/package_header.rs` draws the same arrangement over the same pieces,
//! state by state, and is where the arrangement was settled.
//!
//! # The row is state-driven; the menu is not
//!
//! Every control on the row answers *what does this package need*, so it is the
//! state's own action and nothing else. `Create new revision` answers *what may
//! I choose to do*, which does not vary with state, so it lives in `[⋯]` — and
//! also behind the caret of the `Publish` split button in the states that
//! publish, where it is next to the cursor at the moment it is wanted.
//!
//! # The words are not chosen here
//!
//! Every state label, tone and action comes from `render(state, Site::PageHeader)`,
//! which is the one place the vocabulary lives. A header that hand-wrote its own
//! strings would keep agreeing with itself after the vocabulary moved.
//!
//! # Read-only, and disabled rather than inert
//!
//! This unit draws the header; the commands land with the regions and dialogs
//! that own them. So every command is **disabled and says why** rather than
//! enabled and doing nothing — a control that accepts a click and answers with
//! silence is worse than one that shows it is not available.
//!
//! Two things stay live. The trail, because it is a link and the only way back.
//! And the `[⋯]` trigger, because the arrangement is what this unit is for and a
//! menu that will not open cannot be read.
//!
//! The one real reason a command could carry — `Undo last revision`'s "Nothing
//! has been committed yet" — is replaced by the uniform one while this holds.
//! It would be the lesser truth: the item is unavailable whether or not there
//! is a commit to undo.

use leptos::prelude::*;

use crate::commands;
use crate::kit;
use crate::kit::ActionMenu;
use crate::kit::ActionTone;
use crate::kit::BackLink;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::MenuAction;
use crate::kit::PackageAction;
use crate::kit::SkeletonBox;
use crate::kit::SplitButton;
use crate::kit::SplitOption;
use crate::kit::StateLabel;
use crate::kit::render;
use crate::util;

stylance::import_crate_style!(style, "src/pages/installed_package_v2/header.module.scss");

/// Why every command is unavailable, as the item's `title` and its accessible
/// description. One string, because there is one reason — see the module doc.
const NOT_YET: &str = "Not available on this page yet";

/// The package-level commands. Fixed across states on purpose: the menu is where
/// everything that is *not* the one primary action lives, so it does not change
/// shape as the state does. What varies is what a command can be offered *for* —
/// a catalog link needs a catalog, and an undo needs something to undo.
fn menu(data: &commands::PackageHeaderData) -> Vec<MenuAction> {
    let mut actions = vec![MenuAction {
        label: "Create new revision".to_string(),
        tone: ActionTone::Default,
        disabled: Some(NOT_YET.to_string()),
        on_select: Callback::new(|()| ()),
        separated: false,
    }];

    // Dropped rather than disabled: a package with no catalog has no catalog
    // page, so there is no reason to state and nothing the reader could do.
    if data.uri.as_ref().and_then(util::catalog_url).is_some() {
        actions.push(MenuAction {
            label: "Open in catalog".to_string(),
            tone: ActionTone::Default,
            disabled: Some(NOT_YET.to_string()),
            on_select: Callback::new(|()| ()),
            separated: false,
        });
    }

    // A pushed package is pinned to its push history, so the bucket can be shown
    // and not changed. Same command, honest label — v1's toolbar makes the same
    // swap.
    actions.push(MenuAction {
        label: if data.remote_locked {
            "Show remote".to_string()
        } else {
            "Change bucket".to_string()
        },
        tone: ActionTone::Default,
        disabled: Some(NOT_YET.to_string()),
        on_select: Callback::new(|()| ()),
        separated: false,
    });

    actions.push(MenuAction {
        label: "Undo last revision".to_string(),
        tone: ActionTone::Danger,
        // Its own reason — "Nothing has been committed yet", gated on
        // `has_local_commit` — comes back when the command does. While the page
        // is read-only that reason would be the lesser truth, since the item is
        // unavailable either way.
        disabled: Some(NOT_YET.to_string()),
        on_select: Callback::new(|()| ()),
        separated: true,
    });

    actions.push(MenuAction {
        label: "Remove".to_string(),
        tone: ActionTone::Danger,
        disabled: Some(NOT_YET.to_string()),
        on_select: Callback::new(|()| ()),
        separated: false,
    });

    actions
}

/// The header, for one package.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub fn PageHeader(data: commands::PackageHeaderData) -> impl IntoView {
    let rendered = render(&data.state, kit::Site::PageHeader);
    let action = rendered.action;
    // The states that publish get a split button whose caret holds `Create new
    // revision`; the states with another verb get that verb plainly.
    let publishes = matches!(action, Some(PackageAction::Publish));
    let namespace = data.namespace.to_string();
    let publish_choice = RwSignal::new(0_usize);
    let actions = menu(&data);

    view! {
        <div class=style::root>
            // The parent route, named rather than called "Back": it is an
            // up-link and not history. `/` renders whichever main page is
            // switched on, so it lands a reader back where they came from.
            <BackLink href="/" label="Packages" />
            <div class=style::row>
                <h2 class=style::name>{namespace}</h2>
                <StateLabel tone=rendered.tone>{rendered.words}</StateLabel>

                <div class=style::actions>
                    {publishes
                        .then(|| {
                            view! {
                                <span class=style::action_slot data-primary-action>
                                    <SplitButton
                                        disabled=true
                                        options=vec![
                                            SplitOption::new("Publish", Callback::new(|()| ())),
                                            SplitOption::new(
                                                "Create new revision",
                                                Callback::new(|()| ()),
                                            ),
                                        ]
                                        selected=publish_choice
                                        menu_label="Change what this button does"
                                        variant=ButtonVariant::Primary
                                    />
                                </span>
                            }
                        })}
                    {action
                        .filter(|a| !matches!(a, PackageAction::Publish))
                        .map(|action| {
                            view! {
                                <span class=style::action_slot data-primary-action>
                                    <Button
                                        variant=ButtonVariant::Primary
                                        disabled=true
                                        on_click=|_| ()
                                    >
                                        {action.label()}
                                    </Button>
                                </span>
                            }
                        })}
                    <Button disabled=true on_click=|_| ()>"Open folder"</Button>
                    <ActionMenu
                        aria_label="More actions for this package"
                        actions=actions
                    />
                </div>
            </div>
        </div>
    }
}

/// The header's shape while its read is in flight.
///
/// Built from the same stylesheet as the header itself, so the gap between the
/// two bands is the real one and the height it reserves cannot drift from the
/// header's. A hard-coded pixel height would agree with the header only until
/// somebody changed a control.
#[component]
pub fn PageHeaderSkeleton() -> impl IntoView {
    view! {
        <div class=style::root>
            <SkeletonBox width="88px" height="16px" />
            <div class=style::row>
                <SkeletonBox width="240px" height="var(--q-control-height)" />
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The split button's preference menu, which holds choices and not commands.
    const SPLIT_CHOICES: &str = "[aria-label='Change what this button does']";

    fn data(state: kit::PackageState) -> commands::PackageHeaderData {
        commands::PackageHeaderData {
            namespace: "team/dataset".try_into().unwrap(),
            uri: None,
            state,
            remote_locked: false,
            has_local_commit: false,
            commit_has_parent: false,
        }
    }

    /// The trail names its destination rather than saying "Back", because it is
    /// an up-link to the parent route and not history.
    ///
    /// The chevron is an SVG, so it contributes nothing to `text_content` — the
    /// assertion is on the destination's name, which is the part a reader reads
    /// and the part that would be wrong if this became a history control.
    #[wasm_bindgen_test]
    fn the_trail_names_where_it_goes() {
        let el = mount(|| view! { <PageHeader data=data(kit::PackageState::Latest) /> });
        let link = el
            .query_selector("a[href='/']")
            .unwrap()
            .expect("an up-link home");
        assert_eq!(link.text_content().unwrap().trim(), "Packages");
    }

    #[wasm_bindgen_test]
    fn the_header_names_the_package_and_its_state() {
        let el = mount(|| view! { <PageHeader data=data(kit::PackageState::Behind) /> });
        element_saying(&el, "team/dataset");
        // The one word this site does not borrow from the list, which says
        // "Not the latest".
        element_saying(&el, "Newer revision available");
    }

    /// `Latest` offers nothing, and the row must not collapse when it does —
    /// every state's row is one height, so the page does not jump between them.
    #[wasm_bindgen_test]
    fn the_settled_state_offers_no_action() {
        let el = mount(|| view! { <PageHeader data=data(kit::PackageState::Latest) /> });
        element_saying(&el, "Latest");
        assert!(
            el.query_selector("[data-primary-action]")
                .unwrap()
                .is_none(),
            "nothing to do, so no button; markup was {}",
            el.inner_html()
        );
    }

    /// Read-only by design, stated here so it cannot lapse by accident. A
    /// control that takes a click and answers with silence is worse than one
    /// that shows it is unavailable, so every command is disabled and says why.
    ///
    /// The overflow trigger is the one button that stays live — the arrangement
    /// is what this unit is for, and a menu that will not open cannot be read.
    /// The trail is an `<a>`, so it is not in this sweep at all.
    ///
    /// Both a plain primary and a split one, because they are different controls
    /// and the split has two halves to leave enabled.
    #[wasm_bindgen_test]
    fn every_command_is_disabled_while_the_header_is_read_only() {
        // `Behind` draws a plain primary, `PendingCommit` draws the split one.
        for state in [kit::PackageState::Behind, kit::PackageState::PendingCommit] {
            let el = mount(move || view! { <PageHeader data=data(state.clone()) /> });
            let buttons = el.query_selector_all("button").unwrap();
            let mut live = Vec::new();
            for i in 0..buttons.length() {
                let b: web_sys::Element = buttons.item(i).unwrap().unchecked_into();
                // The split button's own choice list is not a command: the kit's
                // `choices` sets which verb the face shows and explicitly does
                // not run it, so leaving those enabled publishes nothing. Its
                // caret is disabled with the face, so they are unreachable too.
                if b.closest(SPLIT_CHOICES).unwrap().is_some() {
                    continue;
                }
                if !b.has_attribute("disabled") {
                    live.push(b.get_attribute("aria-label").unwrap_or_else(|| {
                        b.text_content().unwrap_or_default().trim().to_string()
                    }));
                }
            }
            assert_eq!(
                live,
                vec!["More actions for this package".to_string()],
                "markup was {}",
                el.inner_html()
            );
        }
    }

    /// The design's rule: the row is state-driven, the menu is not. This command
    /// answers "what may I choose to do", so it is present in every state.
    #[wasm_bindgen_test]
    fn create_new_revision_is_in_the_menu_in_every_state() {
        for state in [
            kit::PackageState::Latest,
            kit::PackageState::Behind,
            kit::PackageState::NoSession { host: None },
        ] {
            let el = mount(move || view! { <PageHeader data=data(state.clone()) /> });
            // Not `element_saying`: a disabled item draws its reason inside the
            // same button, so the button's text is the label followed by it.
            let items = el.query_selector_all("button").unwrap();
            let mut found = false;
            for i in 0..items.length() {
                let b: web_sys::Element = items.item(i).unwrap().unchecked_into();
                if b.closest(SPLIT_CHOICES).unwrap().is_none()
                    && b.text_content()
                        .unwrap_or_default()
                        .starts_with("Create new revision")
                {
                    found = true;
                }
            }
            assert!(found, "markup was {}", el.inner_html());
        }
    }
}

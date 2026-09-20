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
//! # The commands are not wired yet
//!
//! Every item selects into a no-op, and the primary action does not act. This
//! unit draws the header; each command lands with the region that owns it, and
//! wiring them here would put five package mutations behind a surface whose
//! panes do not exist. The one thing that IS live is the trail, which is a
//! link.

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

/// The package-level commands. Fixed across states on purpose: the menu is where
/// everything that is *not* the one primary action lives, so it does not change
/// shape as the state does. What varies is what a command can be offered *for* —
/// a catalog link needs a catalog, and an undo needs something to undo.
fn menu(data: &commands::PackageHeaderData) -> Vec<MenuAction> {
    let mut actions = vec![MenuAction {
        label: "Create new revision".to_string(),
        tone: ActionTone::Default,
        disabled: None,
        on_select: Callback::new(|()| ()),
        separated: false,
    }];

    // Dropped rather than disabled: a package with no catalog has no catalog
    // page, so there is no reason to state and nothing the reader could do.
    if data.uri.as_ref().and_then(util::catalog_url).is_some() {
        actions.push(MenuAction {
            label: "Open in catalog".to_string(),
            tone: ActionTone::Default,
            disabled: None,
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
        disabled: None,
        on_select: Callback::new(|()| ()),
        separated: false,
    });

    actions.push(MenuAction {
        label: "Undo last revision".to_string(),
        tone: ActionTone::Danger,
        // Disabled and stating why, rather than dropped: the command belongs to
        // this package whether or not it has something to undo, and its absence
        // would read as the menu having forgotten it.
        disabled: (!data.has_local_commit).then(|| "Nothing has been committed yet".to_string()),
        on_select: Callback::new(|()| ()),
        separated: true,
    });

    actions.push(MenuAction {
        label: "Remove".to_string(),
        tone: ActionTone::Danger,
        disabled: None,
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
                                    <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                        {action.label()}
                                    </Button>
                                </span>
                            }
                        })}
                    <Button on_click=|_| ()>"Open folder"</Button>
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
    use wasm_bindgen_test::*;

    fn data(state: kit::PackageState) -> commands::PackageHeaderData {
        commands::PackageHeaderData {
            namespace: "team/dataset".try_into().unwrap(),
            uri: None,
            state,
            remote_locked: false,
            has_local_commit: false,
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
            element_saying(&el, "Create new revision");
        }
    }
}

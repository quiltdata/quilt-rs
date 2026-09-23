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
//! # Disabled rather than inert, and the reason is the reader's
//!
//! A control that accepts a click and answers with silence is worse than one
//! that shows it is unavailable, so anything that cannot run is disabled and
//! states why. While a command is running that reason is the same for all of
//! them — one working tree, one command — and it is read reactively, so the
//! menu follows the page rather than whatever was true when it was built.
//!
//! # Where each command reports
//!
//! Three channels — the notification stack, the page's band, a dialog — and
//! one rule, by what the command does. Navigating reports by arriving, on none
//! of them. A self-evident effect says nothing and reports only its failure,
//! on the page's band. `Get latest` keeps what pull already does — its report
//! reaches the notification stack — and its failure goes to the band. A
//! dialog-borne command draws its refusal inside the dialog, which stays open.

use leptos::prelude::*;

use crate::commands;
use crate::kit;
use crate::kit::ActionMenu;
use crate::kit::ActionTone;
use crate::kit::BackLink;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::ConfirmDialog;
use crate::kit::MenuAction;
use crate::kit::PackageAction;
use crate::kit::SkeletonBox;
use crate::kit::SplitButton;
use crate::kit::SplitOption;
use crate::kit::StateLabel;
use crate::kit::Submit;
use crate::kit::render;
use crate::util;

use super::bucket_form::BucketDialog;
use super::role_dialog::RoleDialog;
use super::{Dialogs, Outcome, Wiring};

stylance::import_crate_style!(style, "src/pages/installed_package_v2/header.module.scss");

/// While another command is running. One working tree, one command: two at
/// once is a race the page has no way to arbitrate.
const BUSY: &str = "Something else is running";

/// Which overflow command an item is, so the page can attach a handler to a list
/// the gallery renders inert.
///
/// `pub` because the gallery is a separate binary crate and reaches this one as
/// `quilt_sync_ui::pages::…`; the module stays private behind that re-export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewRevision,
    /// Carries the catalog URL, because whether the item exists at all is the
    /// same question as whether one can be built.
    OpenInCatalog(String),
    /// Change bucket, or Show remote once the package has been pushed.
    Remote,
    Undo,
    Remove,
}

/// One overflow command, as the payload decides it: its words, its tone, whether
/// it sits below a rule, and the reason it cannot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub command: MenuCommand,
    pub label: String,
    pub tone: ActionTone,
    pub separated: bool,
    pub disabled: Option<String>,
}

/// Why undo is unavailable, when it is. `None` means it is available.
fn undo_blocked(data: &commands::PackageHeaderData) -> Option<&'static str> {
    // Order is the engine's: no commit, then the remote guard it applies
    // before it looks at the chain, then the chain's floor.
    if !data.has_local_commit {
        Some("Nothing has been committed yet")
    } else if data.uri.is_some() {
        Some("This package has a remote, so undo is only available before the first push")
    } else if !data.commit_has_parent {
        Some("This is the package's first revision, so there is nothing behind it")
    } else {
        None
    }
}

/// The overflow menu, as a function of the payload and one page fact.
///
/// Fixed across states on purpose: the menu is where everything that is *not*
/// the one primary action lives, so it does not change shape as the state does.
/// What varies is what a command can be offered *for* — a catalog link needs a
/// catalog, and an undo needs something to undo.
pub fn menu_items(data: &commands::PackageHeaderData, busy: bool) -> Vec<MenuItem> {
    let refused = || busy.then(|| BUSY.to_string());
    let mut items = vec![MenuItem {
        command: MenuCommand::NewRevision,
        label: "Create new revision".to_string(),
        tone: ActionTone::Default,
        separated: false,
        disabled: refused(),
    }];

    // Dropped rather than disabled: a package with no catalog has no catalog
    // page, so there is no reason to state and nothing the reader could do.
    if let Some(url) = data.uri.as_ref().and_then(util::catalog_url) {
        items.push(MenuItem {
            command: MenuCommand::OpenInCatalog(url),
            label: "Open in catalog".to_string(),
            tone: ActionTone::Default,
            separated: false,
            disabled: refused(),
        });
    }

    // A pushed package is pinned to its push history, so the bucket can be
    // shown and not changed. Same command, honest label.
    items.push(MenuItem {
        command: MenuCommand::Remote,
        label: if data.remote_locked {
            "Show remote"
        } else {
            "Change bucket"
        }
        .to_string(),
        tone: ActionTone::Default,
        separated: false,
        disabled: refused(),
    });

    items.push(MenuItem {
        command: MenuCommand::Undo,
        label: "Undo last revision".to_string(),
        tone: ActionTone::Danger,
        separated: true,
        // A standing fact outranks a transient one: telling a reader something
        // else is running, when undo would refuse whatever happened, is the
        // lesser truth.
        disabled: undo_blocked(data).map(ToString::to_string).or_else(refused),
    });

    items.push(MenuItem {
        command: MenuCommand::Remove,
        label: "Remove".to_string(),
        tone: ActionTone::Danger,
        separated: false,
        disabled: refused(),
    });

    items
}

/// Run a command, hold the page while it runs, and report only what the band
/// is for.
///
/// Success says nothing here. A command whose success IS worth a sentence —
/// undo — sets its own outcome, because it is the exception rather than the
/// rule, and a helper that reported every success would put `Get latest`'s
/// line on the page behind the toast that already carried its report.
///
/// `on_failure` is the page's own sentence for the command not happening; the
/// backend's text follows it as the detail, which is the split the pause band
/// already makes.
fn run(
    busy: RwSignal<bool>,
    outcome: RwSignal<Option<Outcome>>,
    namespace: String,
    on_failure: &'static str,
    after: Option<Trigger>,
    task: impl std::future::Future<Output = Result<String, String>> + 'static,
) {
    // The controls are disabled while this is true, so this guard only
    // catches a press already in flight when the signal was written.
    if busy.get_untracked() {
        return;
    }
    busy.set(true);
    leptos::task::spawn_local(async move {
        let answer = task.await;
        busy.set(false);
        match answer {
            Ok(_) => {
                if let Some(reload) = after {
                    reload.notify();
                }
            }
            Err(message) => outcome.set(Some(Outcome {
                namespace,
                variant: BannerVariant::Critical,
                lead: on_failure.to_string(),
                detail: Some(message),
            })),
        }
    });
}

/// The overflow menu with a handler on each item.
fn menu(
    data: &commands::PackageHeaderData,
    w: Wiring,
    goto: RwSignal<Option<String>>,
    bucket_open: RwSignal<bool>,
    undo_open: RwSignal<bool>,
    remove_open: RwSignal<bool>,
) -> Vec<MenuAction> {
    let Wiring { busy, outcome, .. } = w;
    let ns = data.namespace.to_string();
    let commit_to = crate::routes::commit_href(&data.namespace);
    menu_items(data, busy.get())
        .into_iter()
        .map(|item| {
            let ns = ns.clone();
            let commit_to = commit_to.clone();
            let on_select = match item.command {
                MenuCommand::NewRevision => {
                    Callback::new(move |()| goto.set(Some(commit_to.clone())))
                }
                MenuCommand::OpenInCatalog(url) => Callback::new(move |()| {
                    let url = url.clone();
                    run(
                        busy,
                        outcome,
                        ns.clone(),
                        "Could not open this package in the catalog.",
                        None,
                        async move { commands::open_in_web_browser(url).await },
                    );
                }),
                MenuCommand::Remote => Callback::new(move |()| bucket_open.set(true)),
                MenuCommand::Undo => Callback::new(move |()| undo_open.set(true)),
                MenuCommand::Remove => Callback::new(move |()| remove_open.set(true)),
            };
            MenuAction {
                label: item.label,
                tone: item.tone,
                disabled: item.disabled,
                on_select,
                separated: item.separated,
            }
        })
        .collect()
}

/// The state's own action, as the row draws it.
///
/// Both shapes live here because they are one slot: the publishing states get a
/// split button whose caret holds `Create new revision`, and the rest get their
/// verb plainly. `Latest` and a denial with no other role held get neither, and
/// the row keeps its height either way.
fn primary_action(
    data: &commands::PackageHeaderData,
    action: Option<PackageAction>,
    w: Wiring,
    goto: RwSignal<Option<String>>,
    publish_choice: RwSignal<usize>,
    bucket_open: RwSignal<bool>,
    role_open: RwSignal<bool>,
) -> AnyView {
    // The state offers no action for a denial — `kit::render` cannot know
    // whether another role is held — so the payload decides, and the verb still
    // comes from the vocabulary.
    let action = data
        .role_switch
        .as_ref()
        .map(|_| PackageAction::SwitchRole)
        .or(action);
    let Wiring {
        busy,
        outcome,
        reload,
        ..
    } = w;
    let ns = data.namespace.clone();
    let uri = data.uri.clone();
    let resolve_to = crate::routes::merge_href(&ns);
    let publish_to = crate::routes::commit_href(&ns);
    let revision_to = publish_to.clone();
    // The deployment to sign in to, when the state names one. `None` for a bare
    // bucket on ambient credentials, which is why the kit offers no action there.
    let sign_in_to = match &data.state {
        kit::PackageState::NoSession { host: Some(host) }
        | kit::PackageState::SignInExpired { host: Some(host) } => {
            Some(crate::routes::sign_in_href(host))
        }
        _ => None,
    };
    if matches!(action, Some(PackageAction::Publish)) {
        return view! {
            <span class=style::action_slot data-primary-action>
                <SplitButton
                    disabled=Signal::derive(move || busy.get())
                    options=vec![
                        SplitOption::new(
                            "Publish",
                            Callback::new(move |()| goto.set(Some(publish_to.clone()))),
                        ),
                        SplitOption::new(
                            "Create new revision",
                            Callback::new(move |()| goto.set(Some(revision_to.clone()))),
                        ),
                    ]
                    selected=publish_choice
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                />
            </span>
        }
        .into_any();
    }

    let on_primary = move |_| match action {
        Some(PackageAction::GetLatest) => {
            let ns = ns.to_string();
            let uri = uri.clone();
            run(
                busy,
                outcome,
                ns.clone(),
                "Could not get the latest revision.",
                Some(reload),
                async move { commands::package_pull(ns, uri).await },
            );
        }
        Some(PackageAction::Resolve) => goto.set(Some(resolve_to.clone())),
        Some(PackageAction::SignIn) => goto.set(sign_in_to.clone()),
        Some(PackageAction::ChooseS3Bucket) => bucket_open.set(true),
        Some(PackageAction::SwitchRole) => role_open.set(true),
        // `Publish` returned above.
        Some(PackageAction::Publish) | None => (),
    };

    action.map_or_else(
        || ().into_any(),
        |action| {
            view! {
                <span class=style::action_slot data-primary-action>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=Signal::derive(move || busy.get())
                        on_click=on_primary
                    >
                        {action.label()}
                    </Button>
                </span>
            }
            .into_any()
        },
    )
}

/// The confirmations behind the menu's two Danger items.
fn danger_dialogs(
    data: &commands::PackageHeaderData,
    w: Wiring,
    goto: RwSignal<Option<String>>,
    undo_open: RwSignal<bool>,
    remove_open: RwSignal<bool>,
) -> impl IntoView + use<> {
    let Wiring {
        outcome, reload, ..
    } = w;
    let ns_undo = data.namespace.to_string();
    let ns_remove = data.namespace.to_string();
    let uri_remove = data.uri.clone();
    view! {
        <ConfirmDialog
            open=undo_open
            title="Undo the last revision"
            consequence="Steps this package back to the revision before its newest one. \
                         This cannot be redone."
            confirm=Submit::new("Undo", move || {
                let ns = ns_undo.clone();
                async move {
                    // No `busy` here, and that is the rule for all four dialogs:
                    // `showModal()` makes the document behind it inert, so there is
                    // nothing outside to disable. The page's signal is for the
                    // commands that run with no dialog holding them.
                    commands::undo_commit(ns.clone()).await?;
                    // The one success the band reports. The state label can read the
                    // same before and after an undo, so the re-read is not a report.
                    outcome.set(Some(Outcome {
                        namespace: ns,
                        variant: BannerVariant::Success,
                        lead: "The last revision was undone.".to_string(),
                        detail: None,
                    }));
                    reload.notify();
                    Ok(())
                }
            })
        />
        <ConfirmDialog
            open=remove_open
            title="Remove this package"
            consequence="Deletes this package's working files, including edits that have \
                         never been committed. The object store keeps committed content only."
            confirm=Submit::new("Remove", move || {
                let ns = ns_remove.clone();
                let uri = uri_remove.clone();
                async move {
                    commands::package_uninstall(ns, uri).await?;
                    // Home, not a refetch: the package this page is about is gone, so
                    // re-reading it would ask for something that no longer exists.
                    // Remove's success is arriving on the package list.
                    goto.set(Some("/".to_string()));
                    Ok(())
                }
            })
        />
    }
}

/// The header, for one package.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub fn PageHeader(data: commands::PackageHeaderData, w: Wiring) -> impl IntoView {
    // The dialogs' flags and `goto` are the page's, not this header's: a
    // re-read rebuilds the header, and must not shut a dialog or drop a
    // navigation on the way — see `Wiring`.
    let Wiring {
        busy,
        outcome,
        goto,
        dialogs,
        ..
    } = w;
    let Dialogs {
        bucket: bucket_open,
        role: role_open,
        undo: undo_open,
        remove: remove_open,
    } = dialogs;
    let payload = StoredValue::new(data.clone());
    let rendered = render(&data.state, kit::Site::PageHeader);
    let action = rendered.action;
    let namespace = data.namespace.to_string();
    let publish_choice = RwSignal::new(0_usize);
    let role_dialog = data.role_switch.clone().map(|switch| {
        view! { <RoleDialog open=role_open switch=switch w=w /> }
    });

    let ns_folder = data.namespace.to_string();
    let uri_folder = data.uri.clone();
    let on_open_folder = move |_| {
        let ns = ns_folder.clone();
        let uri = uri_folder.clone();
        run(
            busy,
            outcome,
            ns.clone(),
            "Could not open this package's folder.",
            None,
            async move { commands::open_in_file_browser(ns, uri).await },
        );
    };

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
                    {primary_action(&data, action, w, goto, publish_choice, bucket_open, role_open)}
                    <Button disabled=Signal::derive(move || busy.get()) on_click=on_open_folder>
                        "Open folder"
                    </Button>
                    // A derived signal, so the items follow `busy` while the menu
                    // stays open: the trigger is live during a run, see
                    // `the_menu_stays_open_when_a_command_settles`.
                    <ActionMenu
                        aria_label="More actions for this package"
                        actions=Signal::derive(move || {
                            menu(
                                &payload.read_value(),
                                w,
                                goto,
                                bucket_open,
                                undo_open,
                                remove_open,
                            )
                        })
                    />
                </div>
            </div>
            <BucketDialog open=bucket_open data=data.clone() w=w />
            {role_dialog}
            {danger_dialogs(&data, w, goto, undo_open, remove_open)}
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
    use crate::kit::BannerVariant;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
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
            role_switch: None,
        }
    }

    /// Mount a header the way the page does — inside a `Router`. Navigation is
    /// the page's (`Wiring::follow`), so only `mount_routed` performs it.
    fn mount_header(data: commands::PackageHeaderData) -> web_sys::Element {
        mount_with(data, Wiring::new())
    }

    fn mount_with(data: commands::PackageHeaderData, w: Wiring) -> web_sys::Element {
        mount(move || {
            view! {
                <Router>
                    <PageHeader data=data.clone() w=w />
                </Router>
            }
        })
    }

    /// Put the browser on an address before the router reads one.
    fn go_to(address: &str) {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
    }

    /// A header mounted on a real route table, so a navigation lands somewhere
    /// this test can read.
    fn mount_routed(data: commands::PackageHeaderData) -> web_sys::Element {
        go_to("/installed-package?namespace=team%2Fdataset");
        mount(move || {
            let data = data.clone();
            let w = Wiring::new();
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                w.follow();
                                view! { <PageHeader data=data.clone() w=w /> }
                            }
                        />
                        <Route path=path!("/commit") view=|| view! { "the commit page" } />
                        <Route path=path!("/merge") view=|| view! { "the merge page" } />
                    </Routes>
                </Router>
            }
        })
    }

    /// The command holding these words. Not `element_saying`: a split button's
    /// choice list repeats the face's label after it in document order, and a
    /// click there sets the default rather than running it.
    fn button(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        let all = el.query_selector_all("button").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|b| {
                b.closest(SPLIT_CHOICES).unwrap().is_none()
                    && b.text_content().unwrap_or_default().trim() == label
            })
            .unwrap_or_else(|| panic!("no command says {label:?}; markup was {}", el.inner_html()))
            .unchecked_into()
    }

    /// The overflow trigger, which is not a command: the arrangement is what the
    /// menu is for, and a menu that will not open cannot be read.
    const TRIGGER: &str = "[aria-label='More actions for this package']";

    /// Every command on screen, by what it says and whether it is refused. A
    /// disabled item draws its reason inside the same button, so the label is a
    /// prefix of the text. The split button's own choice list is excluded — those
    /// set which verb the face shows and explicitly do not run it — and so is the
    /// overflow trigger.
    ///
    /// The menu's items are in the document whether or not it is open —
    /// `AnchoredOverlay` uses the Popover API, so the surface is rendered and
    /// merely not shown, which is why the existing
    /// `create_new_revision_is_in_the_menu_in_every_state` finds them without
    /// opening anything. So this sweep covers the row AND the menu.
    ///
    /// A dialog's footer is left out too: it is in the document while closed,
    /// and its buttons answer the dialog, not the page.
    fn labelled(el: &web_sys::Element) -> Vec<(String, bool)> {
        let all = el.query_selector_all("button").unwrap();
        let mut out = Vec::new();
        for i in 0..all.length() {
            let b: web_sys::Element = all.item(i).unwrap().unchecked_into();
            if b.closest(SPLIT_CHOICES).unwrap().is_some()
                || b.matches(TRIGGER).unwrap()
                || b.closest("dialog").unwrap().is_some()
            {
                continue;
            }
            out.push((
                b.text_content().unwrap_or_default().trim().to_string(),
                b.has_attribute("disabled"),
            ));
        }
        out
    }

    /// The trail names its destination rather than saying "Back", because it is
    /// an up-link to the parent route and not history.
    ///
    /// The chevron is an SVG, so it contributes nothing to `text_content` — the
    /// assertion is on the destination's name, which is the part a reader reads
    /// and the part that would be wrong if this became a history control.
    #[wasm_bindgen_test]
    fn the_trail_names_where_it_goes() {
        let el = mount_header(data(kit::PackageState::Latest));
        let link = el
            .query_selector("a[href='/']")
            .unwrap()
            .expect("an up-link home");
        assert_eq!(link.text_content().unwrap().trim(), "Packages");
    }

    #[wasm_bindgen_test]
    fn the_header_names_the_package_and_its_state() {
        let el = mount_header(data(kit::PackageState::Behind));
        element_saying(&el, "team/dataset");
        // The one word this site does not borrow from the list, which says
        // "Not the latest".
        element_saying(&el, "Newer revision available");
    }

    /// `Latest` offers nothing, and the row must not collapse when it does —
    /// every state's row is one height, so the page does not jump between them.
    #[wasm_bindgen_test]
    fn the_settled_state_offers_no_action() {
        let el = mount_header(data(kit::PackageState::Latest));
        element_saying(&el, "Latest");
        assert!(
            el.query_selector("[data-primary-action]")
                .unwrap()
                .is_none(),
            "nothing to do, so no button; markup was {}",
            el.inner_html()
        );
    }

    /// A command that navigates reports by arriving, and nothing is said —
    /// `channel-per-command`'s first rule. Publishing lands on the commit page,
    /// which is also where `Create new revision` goes: they are one job done two
    /// ways.
    #[wasm_bindgen_test]
    async fn publish_arrives_on_the_commit_page() {
        let el = mount_routed(data(kit::PackageState::PendingCommit));
        sleep_ms(50).await;

        button(&el, "Publish").click();
        sleep_ms(50).await;

        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("the commit page"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// The same rule for the other route. `Diverged` is the one state that
    /// offers it.
    #[wasm_bindgen_test]
    async fn resolve_arrives_on_the_merge_page() {
        let el = mount_routed(data(kit::PackageState::Diverged));
        sleep_ms(50).await;

        button(&el, "Resolve").click();
        sleep_ms(50).await;

        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("the merge page"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// Sign in goes to the deployment the state names, and is offered only where
    /// the state carries one — a bare bucket on ambient AWS credentials has no
    /// deployment to sign in to, which is why `kit::render` gives it no action.
    #[wasm_bindgen_test]
    fn sign_in_is_offered_only_where_the_state_names_a_host() {
        let with_host = mount_header(data(kit::PackageState::NoSession {
            host: Some("demo.quiltdata.com".to_string()),
        }));
        assert!(
            labelled(&with_host)
                .iter()
                .any(|(text, _)| text.starts_with("Sign in")),
            "markup was {}",
            with_host.inner_html()
        );

        let without = mount_header(data(kit::PackageState::NoSession { host: None }));
        assert!(
            without
                .query_selector("[data-primary-action]")
                .unwrap()
                .is_none(),
            "no deployment, no button; markup was {}",
            without.inner_html()
        );
    }

    /// The reactive half of `one-in-flight`, and the defect quilt-rs#974's review
    /// caught: the menu items were built once on open and never updated. Asserted
    /// by flipping the signal AFTER the mount, which a `busy.get()` read at build
    /// time cannot survive.
    #[wasm_bindgen_test]
    async fn every_command_disables_with_a_reason_while_one_runs() {
        let busy = RwSignal::new(false);
        let el = mount_with(
            data(kit::PackageState::Behind),
            Wiring {
                busy,
                ..Wiring::new()
            },
        );

        assert!(
            labelled(&el)
                .iter()
                .any(|(text, off)| text.starts_with("Get latest") && !off),
            "live before anything runs; markup was {}",
            el.inner_html()
        );

        busy.set(true);
        // Render effects run on the next tick, not inside `set`.
        leptos::task::tick().await;

        let after = labelled(&el);
        assert!(
            after.iter().all(|(_, off)| *off),
            "every command is refused while one runs: {after:?}"
        );
        assert!(
            after
                .iter()
                .any(|(text, _)| text.contains("Something else is running")),
            "and says why: {after:?}"
        );
    }

    /// The trigger stays live while a command runs, so the reasons can be read.
    /// It is not a command — the arrangement is what the menu is for.
    #[wasm_bindgen_test]
    fn the_overflow_trigger_stays_live_while_a_command_runs() {
        let el = mount_with(
            data(kit::PackageState::Behind),
            Wiring {
                busy: RwSignal::new(true),
                ..Wiring::new()
            },
        );
        let trigger = el
            .query_selector(TRIGGER)
            .unwrap()
            .expect("the overflow trigger");
        assert!(!trigger.has_attribute("disabled"));
    }

    /// The menu can be opened while a command runs, to read why its items are
    /// refused; the command settling must not shut it on the reader.
    #[wasm_bindgen_test]
    async fn the_menu_stays_open_when_a_command_settles() {
        let busy = RwSignal::new(true);
        let el = mount_with(
            data(kit::PackageState::Behind),
            Wiring {
                busy,
                ..Wiring::new()
            },
        );
        open_menu(&el);
        leptos::task::tick().await;

        busy.set(false);
        leptos::task::tick().await;

        let trigger = el
            .query_selector(TRIGGER)
            .unwrap()
            .expect("the overflow trigger");
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("true"),
            "still open; markup was {}",
            el.inner_html()
        );
        assert!(
            !menu_item(&el, "Remove").disabled(),
            "and the items followed"
        );
    }

    /// There is no Tauri bridge under the runner, so `invoke` answers `Err` —
    /// which makes this the failure arm, and the failure arm is the one the band
    /// exists for. The lead is the page's sentence; the bridge's own text follows
    /// as the detail.
    #[wasm_bindgen_test]
    async fn a_failed_open_folder_reports_on_the_band_and_names_the_package() {
        let outcome: RwSignal<Option<Outcome>> = RwSignal::new(None);
        let el = mount_with(
            data(kit::PackageState::Latest),
            Wiring {
                busy: RwSignal::new(false),
                outcome,
                ..Wiring::new()
            },
        );

        button(&el, "Open folder").click();
        sleep_ms(50).await;

        let said = outcome
            .get_untracked()
            .expect("the failure reached the band");
        assert_eq!(
            said.namespace, "team/dataset",
            "keyed to the package it was for"
        );
        assert_eq!(said.variant, BannerVariant::Critical);
        assert_eq!(said.lead, "Could not open this package's folder.");
        assert!(
            said.detail.is_some(),
            "and the bridge's own words follow it"
        );
    }

    /// Get latest's failure is the band's; its success is the notification
    /// stack's, through pull's own report. And the in-flight signal comes back
    /// down when the command settles, or the page would stay sealed after one
    /// failure.
    #[wasm_bindgen_test]
    async fn get_latest_reports_its_failure_on_the_band() {
        let outcome: RwSignal<Option<Outcome>> = RwSignal::new(None);
        let busy = RwSignal::new(false);
        let el = mount_with(
            data(kit::PackageState::Behind),
            Wiring {
                busy,
                outcome,
                ..Wiring::new()
            },
        );

        button(&el, "Get latest").click();
        sleep_ms(50).await;

        let said = outcome
            .get_untracked()
            .expect("the failure reached the band");
        assert_eq!(said.lead, "Could not get the latest revision.");
        assert!(
            !busy.get_untracked(),
            "the signal comes back down when it settles"
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
            let el = mount_header(data(state));
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

    /// Both entries open the same form — the row offers it because the state
    /// calls for it, the menu because the reader may choose it.
    #[wasm_bindgen_test]
    async fn choose_s3_bucket_and_change_bucket_open_the_same_dialog() {
        for state in [kit::PackageState::NoRemote, kit::PackageState::Latest] {
            let from_row = matches!(state, kit::PackageState::NoRemote);
            let el = mount_header(data(state));
            assert!(
                el.query_selector("dialog[open]").unwrap().is_none(),
                "closed until asked"
            );

            if from_row {
                button(&el, "Choose S3 bucket").click();
            } else {
                el.query_selector(TRIGGER)
                    .unwrap()
                    .expect("the overflow trigger")
                    .unchecked_into::<web_sys::HtmlElement>()
                    .click();
                leptos::task::tick().await;
                button(&el, "Change bucket").click();
            }
            // The dialog opens from an effect, on the next tick.
            leptos::task::tick().await;

            let dialog = el
                .query_selector("dialog[open]")
                .unwrap()
                .unwrap_or_else(|| panic!("the dialog opened; markup was {}", el.inner_html()));
            assert!(
                dialog
                    .text_content()
                    .unwrap_or_default()
                    .contains("Change bucket"),
                "markup was {}",
                dialog.inner_html()
            );
        }
    }

    /// A single-role reader sees the reason and no button — the payload says so
    /// by carrying no remedy, and the header must not invent one.
    #[wasm_bindgen_test]
    fn a_denial_with_no_alternatives_offers_no_button() {
        let el = mount_header(data(kit::PackageState::RoleDenied {
            role: Some("analyst".to_string()),
        }));
        element_saying(&el, "No access");
        assert!(
            el.query_selector("[data-primary-action]")
                .unwrap()
                .is_none(),
            "markup was {}",
            el.inner_html()
        );
    }

    /// And the same denial with another role held offers the remedy.
    #[wasm_bindgen_test]
    async fn a_denial_with_an_alternative_offers_switch_role() {
        let mut d = data(kit::PackageState::RoleDenied {
            role: Some("analyst".to_string()),
        });
        d.role_switch = Some(commands::RoleSwitch {
            host: "demo.quiltdata.com".to_string(),
            alternatives: vec!["admin".to_string()],
        });
        let el = mount_header(d);

        button(&el, "Switch role").click();
        // The dialog opens from an effect, on the next tick.
        leptos::task::tick().await;
        let dialog = el
            .query_selector("dialog[open]")
            .unwrap()
            .expect("the role dialog");
        assert!(
            dialog.text_content().unwrap_or_default().contains("admin"),
            "the alternatives are the options; markup was {}",
            dialog.inner_html()
        );
        assert!(
            !dialog
                .text_content()
                .unwrap_or_default()
                .contains("analyst"),
            "and the refused role is not one of them"
        );
    }

    /// A refused switch names the role it was refused, the way a refused
    /// remote names its bucket. There is no bridge under the runner, so the
    /// refusal is what runs.
    #[wasm_bindgen_test]
    async fn a_refused_switch_names_the_role() {
        let mut d = data(kit::PackageState::RoleDenied {
            role: Some("analyst".to_string()),
        });
        d.role_switch = Some(commands::RoleSwitch {
            host: "demo.quiltdata.com".to_string(),
            alternatives: vec!["admin".to_string()],
        });
        let el = mount_header(d);

        button(&el, "Switch role").click();
        leptos::task::tick().await;
        button(&el, "Switch").click();
        sleep_ms(50).await;

        let alert = el
            .query_selector("dialog[open] [role=alert]")
            .unwrap()
            .unwrap_or_else(|| {
                panic!("the refusal, in the dialog; markup was {}", el.inner_html())
            });
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains("Could not switch to admin: "),
            "markup was {}",
            alert.inner_html()
        );
    }

    /// The overflow menu's surface. The trigger shares its name, so the popover
    /// attribute is what tells the list from the button that opens it.
    const SURFACE: &str = "[popover][aria-label='More actions for this package']";

    /// Open `[⋯]` the way a reader does.
    fn open_menu(el: &web_sys::Element) {
        el.query_selector(TRIGGER)
            .unwrap()
            .expect("the overflow trigger")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
    }

    /// The menu's items, each with its label apart from any reason drawn inside it.
    fn menu_entries(el: &web_sys::Element) -> Vec<(String, web_sys::Element)> {
        let all = el.query_selector_all(&format!("{SURFACE} button")).unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .map(|b| {
                let text = b.text_content().unwrap_or_default();
                let reason = b
                    .query_selector("span")
                    .unwrap()
                    .and_then(|s| s.text_content())
                    .unwrap_or_default();
                let label = text
                    .strip_suffix(reason.as_str())
                    .unwrap_or(&text)
                    .trim()
                    .to_string();
                (label, b)
            })
            .collect()
    }

    /// The menu item with this label. A disabled item draws its reason inside
    /// the same button, so the label is matched on its own.
    fn menu_item(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        menu_entries(el)
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| {
                panic!(
                    "no menu item says {label:?}; markup was {}",
                    el.inner_html()
                )
            })
            .1
            .unchecked_into()
    }

    /// The labels of the menu's Danger items, read off the kit's own variant
    /// marker as `confirm_dialog.rs` reads it off the Button.
    fn menu_labels_with_tone_danger(el: &web_sys::Element) -> Vec<String> {
        menu_entries(el)
            .into_iter()
            .filter(|(_, b)| b.class_name().contains("danger"))
            .map(|(label, _)| label)
            .collect()
    }

    /// The open dialog's footer, in document order. Scoped to `[open]` because
    /// the header holds four dialogs and `Dialog` renders all of their children.
    fn footer_labels(el: &web_sys::Element) -> Vec<String> {
        let all = el.query_selector_all("dialog[open] button").unwrap();
        (0..all.length())
            .filter_map(|i| all.item(i).unwrap().text_content())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// A Danger item picks the command; the dialog accepts the consequence. The
    /// press must not run anything — which is the defect quilt-rs#974 shipped,
    /// with Remove firing straight from the menu.
    #[wasm_bindgen_test]
    async fn remove_asks_before_it_runs_and_says_what_it_deletes() {
        let el = mount_header(data(kit::PackageState::Latest));
        open_menu(&el);
        menu_item(&el, "Remove").click();
        sleep_ms(20).await;

        let dialog = el
            .query_selector("dialog[open]")
            .unwrap()
            .expect("the confirmation");
        let words = dialog.text_content().unwrap_or_default();
        assert!(
            words.contains("including edits that have never been committed"),
            "the consequence names what is lost: {words}"
        );
        assert_eq!(
            footer_labels(&el),
            vec!["Cancel".to_string(), "Remove".to_string()]
        );
    }

    /// Undo confirms too, for a different reason: it destroys nothing, and has
    /// no redo.
    #[wasm_bindgen_test]
    async fn undo_asks_before_it_runs_and_says_it_cannot_be_redone() {
        let mut d = data(kit::PackageState::Latest);
        d.has_local_commit = true;
        d.commit_has_parent = true;
        let el = mount_header(d);
        open_menu(&el);
        menu_item(&el, "Undo last revision").click();
        sleep_ms(20).await;

        let dialog = el
            .query_selector("dialog[open]")
            .unwrap()
            .expect("the confirmation");
        assert!(
            dialog
                .text_content()
                .unwrap_or_default()
                .contains("cannot be redone")
        );
        assert_eq!(
            footer_labels(&el),
            vec!["Cancel".to_string(), "Undo".to_string()]
        );
    }

    /// The transient refusal — a dirty tree — is drawn where it is discovered,
    /// inside the dialog, and the dialog stays open. There is no bridge under
    /// the runner, so the failure arm is what runs, which is the arm this claim
    /// is about.
    #[wasm_bindgen_test]
    async fn a_refused_undo_stays_in_the_dialog_and_never_reaches_the_band() {
        let outcome: RwSignal<Option<Outcome>> = RwSignal::new(None);
        let mut d = data(kit::PackageState::Latest);
        d.has_local_commit = true;
        d.commit_has_parent = true;
        let el = mount_with(
            d,
            Wiring {
                busy: RwSignal::new(false),
                outcome,
                ..Wiring::new()
            },
        );
        open_menu(&el);
        menu_item(&el, "Undo last revision").click();
        sleep_ms(20).await;
        button(&el, "Undo").click();
        sleep_ms(50).await;

        let dialog = el
            .query_selector("dialog[open]")
            .unwrap()
            .expect("still open");
        assert!(
            dialog.query_selector("[role=alert]").unwrap().is_some(),
            "the reason is inside the dialog; markup was {}",
            dialog.inner_html()
        );
        assert!(
            outcome.get_untracked().is_none(),
            "and nothing reached the band"
        );
    }

    /// The three standing reasons reach the item, and an available undo is live.
    #[wasm_bindgen_test]
    fn the_undo_item_carries_the_reason_the_payload_gives_it() {
        let mut blocked = data(kit::PackageState::Latest);
        blocked.has_local_commit = false;
        let el = mount_header(blocked);
        open_menu(&el);
        assert!(
            menu_item(&el, "Undo last revision")
                .text_content()
                .unwrap_or_default()
                .contains("Nothing has been committed yet"),
        );

        let mut available = data(kit::PackageState::Latest);
        available.has_local_commit = true;
        available.commit_has_parent = true;
        let el = mount_header(available);
        open_menu(&el);
        assert!(!menu_item(&el, "Undo last revision").disabled());
    }

    /// Nothing else on this page confirms. A third `ConfirmDialog` would be a
    /// tone rule nobody decided.
    #[wasm_bindgen_test]
    fn only_the_two_danger_items_are_followed_by_a_confirmation() {
        let el = mount_header(data(kit::PackageState::Latest));
        open_menu(&el);
        let danger: Vec<String> = menu_labels_with_tone_danger(&el);
        assert_eq!(
            danger,
            vec!["Undo last revision".to_string(), "Remove".to_string()],
        );
    }

    /// Undo's three refusals, told apart because the reader can act on the
    /// difference — commit something, or accept that a pushed package has no
    /// chain left. A single reason would make the first look like the third.
    /// The fourth refusal, a dirty tree, is deliberately absent: it is transient,
    /// and the engine states it when the command runs.
    #[test]
    fn undo_names_which_of_the_three_standing_facts_blocks_it() {
        let uri: quilt_uri::S3PackageUri = "quilt+s3://team-bucket#package=team/dataset"
            .parse()
            .expect("a uri");
        let cases = [
            (false, false, None, Some("Nothing has been committed yet")),
            (
                true,
                true,
                Some(uri),
                Some("This package has a remote, so undo is only available before the first push"),
            ),
            (
                true,
                false,
                None,
                Some("This is the package's first revision, so there is nothing behind it"),
            ),
            (true, true, None, None),
        ];
        for (has_commit, has_parent, uri, expected) in cases {
            let mut d = data(kit::PackageState::Latest);
            d.has_local_commit = has_commit;
            d.commit_has_parent = has_parent;
            d.uri = uri;
            assert_eq!(undo_blocked(&d), expected);
        }
    }

    /// A package with no catalog has no catalog page, so the item is dropped
    /// rather than disabled: there is no reason to state and nothing the reader
    /// could do about it.
    #[test]
    fn open_in_catalog_is_absent_without_a_catalog_rather_than_disabled() {
        let plain = data(kit::PackageState::Latest);
        assert!(
            !menu_items(&plain, false)
                .iter()
                .any(|item| matches!(item.command, MenuCommand::OpenInCatalog(_))),
            "no catalog host, no item"
        );

        let mut with_catalog = data(kit::PackageState::Latest);
        with_catalog.uri = Some(
            "quilt+s3://team-bucket#package=team/dataset&catalog=open.quiltdata.com"
                .parse()
                .expect("a uri with a catalog"),
        );
        assert!(
            menu_items(&with_catalog, false)
                .iter()
                .any(|item| matches!(item.command, MenuCommand::OpenInCatalog(_))),
        );
    }
}

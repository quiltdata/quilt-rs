//! The v2 package page's header: trail, identity, one resolved state, one
//! action, `Open folder`, and everything else behind `[⋯]`.
//!
//! # The row is the kit's, the contents are this page's
//!
//! The geometry — trail, name, label, trailing actions — is `kit::PageHeader`,
//! which the commit page draws too. What fills it is this page's: the
//! package's state, its action, its menu and the dialogs behind them. The
//! gallery scene at `gallery/package_header.rs` draws the same row over the
//! same pieces, state by state, and is where the arrangement was settled.
//!
//! # The row is state-driven; the menu is not
//!
//! Every control on the row answers *what does this package need*, so it is the
//! state's own action and nothing else. `Create new revision` answers *what may
//! I choose to do*, which does not vary with state, so it lives in `[⋯]`. The
//! caret of the `Publish` split button holds `Review before publishing…`, which
//! is the same commit page reached from the other side: the moment the reader
//! wants to check the message and metadata is the moment they are about to
//! publish, so that is where the cursor already is.
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
//! A dialog's command holds the same signal. The signal is the page's and the
//! dialog's own seal is not, so a re-read that rebuilds the header mid-submit
//! draws the new dialog sealed instead of ready to submit a second time.
//!
//! # Where each command reports
//!
//! Three channels — the notification stack, the page's band, a dialog — and
//! one rule, by what the command does. Navigating reports by arriving, on none
//! of them. A self-evident effect says nothing and reports only its failure,
//! on the page's band. `Get latest` and `Publish` keep what pull and publish
//! already do — their report reaches the notification stack — and their
//! failure goes to the band. A dialog-borne command draws its refusal inside
//! the dialog, which stays open. Every command, as it starts, retracts what the
//! band said about the last one.
//!
//! `Publish` runs in place, with the message, workflow and metadata from the
//! publish settings, so the package's most common job is one click. The page
//! is read again after it whether it succeeds or not, because it commits
//! before it pushes and a refused push still leaves a new local revision.
//! `Review before publishing…` behind its caret and `Create new revision` in
//! `[⋯]` navigate to the commit page and report by arriving: choosing what the
//! revision says is the point of them.
//!
//! # The caret's choice is remembered, per package
//!
//! Picking `Review before publishing…` behind the caret puts it on the face,
//! and the face stays that way for that package the next time the page opens.
//! Per package, because a package that needs its message checked every time
//! is not a reason to slow down every other one. It is a choice the reader
//! made deliberately, not one inferred from what they did last — the line the
//! kit's split button draws for whoever persists it.
//!
//! It lives in the webview's `localStorage`, as a name (`publish`, `review`)
//! rather than the option's index or label, so reordering or relabelling the
//! options cannot flip what a stored choice means. Anything unreadable falls
//! back to `Publish`, and a failure to read or write says nothing: the cost of
//! losing it is one extra click.

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
use crate::kit::ConfirmOption;
use crate::kit::MenuAction;
use crate::kit::PackageAction;
use crate::kit::SplitButton;
use crate::kit::SplitOption;
use crate::kit::StateLabel;
use crate::kit::Submit;
use crate::kit::render;
use crate::util;

use super::bucket_form::BucketDialog;
use super::role_dialog::RoleDialog;
use super::{Dialogs, Outcome, Reread, Wiring, holding, run};

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
                        Reread::Never,
                        async move { commands::open_in_web_browser(url).await },
                    );
                }),
                MenuCommand::Remote => Callback::new(move |()| bucket_open.set(true)),
                MenuCommand::Undo => Callback::new(move |()| undo_open.set(true)),
                MenuCommand::Remove => Callback::new(move |()| {
                    // Checked each time it opens: an unchecked box from an
                    // earlier opening is not this one's answer.
                    w.dialogs.remove_prune.set(true);
                    remove_open.set(true);
                }),
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

/// The stored names of the `Publish` split button's options, in the order
/// [`primary_action`] lists them: the stored value is the name, the signal the
/// kit reads is the index.
const PUBLISH_CHOICES: [&str; 2] = ["publish", "review"];

/// The `localStorage` key a package's choice lives under.
fn publish_choice_key(namespace: &str) -> String {
    format!("quilt-sync.publish-choice.{namespace}")
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The option this package's split button last had on its face. `Publish`
/// when nothing usable is stored, or there is no storage to read.
fn remembered_publish_choice(namespace: &str) -> usize {
    local_storage()
        .and_then(|storage| storage.get_item(&publish_choice_key(namespace)).ok()?)
        .and_then(|name| PUBLISH_CHOICES.iter().position(|known| *known == name))
        .unwrap_or(0)
}

/// Store the option the reader picked for this package. Every failure is
/// ignored — a quota, a private mode, no storage at all.
fn remember_publish_choice(namespace: &str, choice: usize) {
    let (Some(storage), Some(name)) = (local_storage(), PUBLISH_CHOICES.get(choice)) else {
        return;
    };
    drop(storage.set_item(&publish_choice_key(namespace), name));
}

/// The state's own action, as the row draws it.
///
/// Both shapes live here because they are one slot: the publishing states get a
/// split button whose caret holds `Review before publishing…`, and the rest get their
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
    // Keeps a deep link's mismatch, so entering the mode does not end its band.
    let resolve_to = super::carrying(crate::routes::resolve_href(&ns));
    let revision_to = crate::routes::commit_href(&ns);
    // `Publish` is pull's shape — run here, reported by its toast — except that
    // the page is read again after a failure as well: the commit can land and
    // the push still be refused, and the page must show the revision that now
    // exists.
    let on_publish = {
        let ns = ns.to_string();
        let uri = uri.clone();
        Callback::new(move |()| {
            let ns = ns.clone();
            let uri = uri.clone();
            run(
                busy,
                outcome,
                ns.clone(),
                "Could not publish this package.",
                Reread::Always(reload),
                async move { commands::package_publish(ns, uri).await },
            );
        })
    };
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
                    // In `PUBLISH_CHOICES`' order, which is what a stored choice names.
                    options=vec![
                        SplitOption::new("Publish", on_publish),
                        SplitOption::new(
                            "Review before publishing…",
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
                Reread::OnSuccess(reload),
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
        busy,
        outcome,
        reload,
        dialogs,
        ..
    } = w;
    let prune = dialogs.remove_prune;
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
                    holding(busy, outcome, commands::undo_commit(ns.clone())).await?;
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
            running=busy
        />
        <ConfirmDialog
            open=remove_open
            title="Remove this package"
            // No promise about the object store: a commit never pushed is named
            // only by the manifests uninstall deletes, so it is lost either way.
            consequence="Deletes this package's working files, including edits and commits \
                         that were never pushed."
            option=ConfirmOption::new("Also delete downloaded files from disk", prune)
            confirm=Submit::new("Remove", move || {
                let ns = ns_remove.clone();
                let uri = uri_remove.clone();
                let prune = prune.get_untracked();
                async move {
                    // What the prune freed arrives as a toast, which outlives
                    // the move home.
                    holding(busy, outcome, commands::package_uninstall(ns, uri, prune)).await?;
                    // Home, not a refetch: the package this page is about is gone, so
                    // re-reading it would ask for something that no longer exists.
                    // Remove's success is arriving on the package list.
                    goto.set(Some("/".to_string()));
                    Ok(())
                }
            })
            running=busy
        />
    }
}

/// The header, for one package.
///
/// `resolving` is the page's one `open`: while the resolve mode is open the
/// pane holds the choices, so the primary is hidden and the rest stays.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub fn PageHeader(
    data: commands::PackageHeaderData,
    w: Wiring,
    #[prop(optional, into)] resolving: Signal<bool>,
) -> impl IntoView {
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
        // The bucket dialog's own.
        bucket_draft: _,
        role: role_open,
        undo: undo_open,
        remove: remove_open,
        // The menu resets it, and the dialog reads it, through `w`.
        remove_prune: _,
        // The resolve pane's confirmation, not the header's.
        replace: _,
    } = dialogs;
    let payload = StoredValue::new(data.clone());
    let rendered = render(&data.state, kit::Site::PageHeader);
    let action = rendered.action;
    let namespace = data.namespace.to_string();
    // Seeded from what the reader picked for this package last time, and
    // written back only when they pick: a visit that changes nothing stores
    // nothing.
    let seed = remembered_publish_choice(&namespace);
    let publish_choice = RwSignal::new(seed);
    {
        let namespace = namespace.clone();
        // Compared with the last value seen rather than watched for changes:
        // an effect's first run comes a tick after the header is drawn, and a
        // pick in that tick must still be stored.
        Effect::new(move |seen: Option<usize>| {
            let choice = publish_choice.get();
            if choice != seen.unwrap_or(seed) {
                remember_publish_choice(&namespace, choice);
            }
            choice
        });
    }
    // The role the denial named, for the dialog's sentence. The remedy is only
    // carried with a denial, so any other state has no role to name.
    let refused = match &data.state {
        kit::PackageState::RoleDenied { role } => role.clone(),
        _ => None,
    };
    let role_dialog = data.role_switch.clone().map(|switch| {
        view! { <RoleDialog open=role_open switch=switch refused=refused w=w /> }
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
            Reread::Never,
            async move { commands::open_in_file_browser(ns, uri).await },
        );
    };

    view! {
        <div>
            <kit::PageHeader
                // The parent route, named rather than called "Back": it is an
                // up-link and not history. `/` renders whichever main page is
                // switched on, so it lands a reader back where they came from.
                trail=view! { <BackLink href="/" label="Packages" /> }.into_any()
                title=namespace
                label=view! { <StateLabel tone=rendered.tone>{rendered.words}</StateLabel> }
                    .into_any()
                actions=view! {
                    {move || {
                        (!resolving.get())
                            .then(|| {
                                primary_action(
                                    &payload.read_value(),
                                    action,
                                    w,
                                    goto,
                                    publish_choice,
                                    bucket_open,
                                    role_open,
                                )
                            })
                    }}
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
                }
                    .into_any()
            />
            <BucketDialog open=bucket_open data=data.clone() w=w />
            {role_dialog}
            {danger_dialogs(&data, w, goto, undo_open, remove_open)}
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
        mount_routed_with(data, Wiring::new())
    }

    /// [`mount_routed`] over the page's signals, so a test can read the band.
    fn mount_routed_with(data: commands::PackageHeaderData, w: Wiring) -> web_sys::Element {
        go_to("/installed-package?namespace=team%2Fdataset");
        mount(move || {
            let data = data.clone();
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                w.follow(Signal::stored("team/dataset".to_string()));
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

    /// `Publish` runs here, the way `Get latest` does: the page holds
    /// while it runs and stays where it is. There is no bridge under the
    /// runner, so the failure arm is what runs — a workflow's refusal takes the
    /// same path — and it reaches the band, keyed to the package.
    ///
    /// The page is read again after the failure too: publish commits before it
    /// pushes, so a refused push can leave a revision the old details do not
    /// show. The re-read leaves the band alone.
    #[wasm_bindgen_test]
    async fn publish_runs_in_place_and_reports_its_failure_on_the_band() {
        forget_choices();
        let w = Wiring::new();
        let reloads = RwSignal::new(0_u32);
        let owner = Owner::new();
        owner.with(|| {
            Effect::new(move |seen: Option<()>| {
                w.reload.track();
                // The first run is the subscription, not a re-read.
                if seen.is_some() {
                    reloads.update(|n| *n += 1);
                }
            });
        });
        let el = mount_routed_with(data(kit::PackageState::PendingCommit), w);
        sleep_ms(50).await;

        button(&el, "Publish").click();
        sleep_ms(50).await;

        assert_eq!(reloads.get_untracked(), 1, "read again after the failure");

        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("the commit page"),
            "stays on the package page; markup was {}",
            el.inner_html()
        );
        let said = w
            .outcome
            .get_untracked()
            .expect("the failure reached the band");
        assert_eq!(said.namespace, "team/dataset");
        assert_eq!(said.variant, BannerVariant::Critical);
        assert_eq!(said.lead, "Could not publish this package.");
        assert!(
            !w.busy.get_untracked(),
            "the signal comes back down when it settles"
        );
    }

    /// `Review before publishing…` behind the caret goes to the commit page,
    /// where the reader checks the message and metadata first, and reports by
    /// arriving — `channel-per-command`'s first rule.
    #[wasm_bindgen_test]
    async fn review_before_publishing_on_the_caret_arrives_on_the_commit_page() {
        forget_choices();
        let w = Wiring::new();
        let el = mount_routed_with(data(kit::PackageState::PendingCommit), w);
        sleep_ms(50).await;

        pick(&el, "Review before publishing…");
        leptos::task::tick().await;
        // The face now says it, and a press on the face runs it.
        button(&el, "Review before publishing…").click();
        sleep_ms(50).await;

        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("the commit page"),
            "markup was {}",
            el.inner_html()
        );
        assert!(w.outcome.get_untracked().is_none(), "and nothing is said");
        forget_choices();
    }

    /// A namespace no other test draws, for the choice that must not leak.
    const OTHER: &str = "team/other";

    /// Every key these tests store under, removed, so no test starts on a face
    /// another one picked.
    fn forget_choices() {
        let storage = local_storage().expect("a browser with storage");
        for namespace in ["team/dataset", OTHER] {
            drop(storage.remove_item(&publish_choice_key(namespace)));
        }
    }

    fn stored_choice(namespace: &str) -> Option<String> {
        local_storage()
            .expect("a browser with storage")
            .get_item(&publish_choice_key(namespace))
            .unwrap()
    }

    fn store_choice(namespace: &str, value: &str) {
        local_storage()
            .expect("a browser with storage")
            .set_item(&publish_choice_key(namespace), value)
            .unwrap();
    }

    /// Pick `label` behind the caret: it moves the face and runs nothing.
    fn pick(el: &web_sys::Element, label: &str) {
        let choices = el
            .query_selector_all(&format!("[popover]{SPLIT_CHOICES} button"))
            .unwrap();
        (0..choices.length())
            .map(|i| {
                choices
                    .item(i)
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlElement>()
            })
            .find(|b| b.text_content().unwrap_or_default().contains(label))
            .unwrap_or_else(|| panic!("the caret's choice; markup was {}", el.inner_html()))
            .click();
    }

    /// A package whose reader picked `Review before publishing…` opens with it
    /// on the face.
    #[wasm_bindgen_test]
    fn a_remembered_review_is_on_the_face() {
        forget_choices();
        store_choice("team/dataset", "review");

        let el = mount_header(data(kit::PackageState::PendingCommit));
        button(&el, "Review before publishing…");

        forget_choices();
    }

    /// Picking stores the choice for that package and no other: a package
    /// that needs reviewing does not slow down the rest.
    #[wasm_bindgen_test]
    async fn picking_remembers_the_choice_for_that_package_only() {
        forget_choices();
        let el = mount_header(data(kit::PackageState::PendingCommit));
        assert_eq!(
            stored_choice("team/dataset"),
            None,
            "opening stores nothing"
        );

        pick(&el, "Review before publishing…");
        sleep_ms(10).await;
        button(&el, "Review before publishing…");
        assert_eq!(stored_choice("team/dataset").as_deref(), Some("review"));
        assert_eq!(stored_choice(OTHER), None);

        let other = commands::PackageHeaderData {
            namespace: OTHER.try_into().unwrap(),
            ..data(kit::PackageState::PendingCommit)
        };
        let el = mount_header(other);
        button(&el, "Publish");

        forget_choices();
    }

    /// What storage holds is not trusted: a name this build does not know —
    /// an older build's, an index, a hand edit — falls back to `Publish`.
    #[wasm_bindgen_test]
    fn an_unknown_stored_choice_falls_back_to_publish() {
        forget_choices();
        for garbage in ["1", "Review before publishing…", "", "nonsense"] {
            store_choice("team/dataset", garbage);
            assert_eq!(remembered_publish_choice("team/dataset"), 0, "{garbage:?}");
        }
        store_choice("team/dataset", "nonsense");
        let el = mount_header(data(kit::PackageState::PendingCommit));
        button(&el, "Publish");

        forget_choices();
    }

    /// Resolve is a mode of this page, not another route: it pushes `resolve=1`
    /// on this package's address, so Back leaves the mode. `mount_routed` keeps
    /// `/merge`, so a regression to the merge page shows here.
    #[wasm_bindgen_test]
    async fn resolve_opens_the_mode_on_this_page() {
        let el = mount_routed(data(kit::PackageState::Diverged));
        sleep_ms(50).await;
        let window = web_sys::window().unwrap();
        let before = window.history().unwrap().length().unwrap();

        button(&el, "Resolve").click();
        sleep_ms(50).await;

        assert_eq!(
            window.location().search().unwrap(),
            "?namespace=team%2Fdataset&filter=unmodified&resolve=1",
        );
        assert_eq!(
            window.history().unwrap().length().unwrap(),
            before + 1,
            "pushed, not replaced"
        );
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("the merge page"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// While the mode is open the pane holds the two choices, so the header's
    /// primary would be a second way in to where the reader already is. The
    /// state, `Open folder` and the menu stay.
    #[wasm_bindgen_test]
    fn while_the_mode_is_open_the_header_offers_no_primary() {
        let el = mount(move || {
            view! {
                <Router>
                    <PageHeader
                        data=data(kit::PackageState::Diverged)
                        w=Wiring::new()
                        resolving=Signal::stored(true)
                    />
                </Router>
            }
        });
        assert!(
            el.query_selector("[data-primary-action]")
                .unwrap()
                .is_none(),
            "no primary in the mode; markup was {}",
            el.inner_html()
        );
        button(&el, "Open folder");
        assert!(
            el.query_selector("[aria-label='More actions for this package']")
                .unwrap()
                .is_some(),
            "the menu stays; markup was {}",
            el.inner_html()
        );
        element_saying(
            &el,
            &render(&kit::PackageState::Diverged, kit::Site::PageHeader).words,
        );
    }

    #[wasm_bindgen_test]
    async fn leaving_the_mode_brings_resolve_back() {
        let resolving = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <Router>
                    <PageHeader
                        data=data(kit::PackageState::Diverged)
                        w=Wiring::new()
                        resolving=resolving
                    />
                </Router>
            }
        });
        resolving.set(false);
        leptos::task::tick().await;
        button(&el, "Resolve");
    }

    /// Sign in goes to the deployment the state names, and is offered only where
    /// the state carries one — a bare bucket on ambient AWS credentials has no
    /// deployment to sign in to, which is why `kit::render` gives it no action.
    #[wasm_bindgen_test]
    fn sign_in_is_offered_only_where_the_state_names_a_host() {
        let with_host = mount_header(data(kit::PackageState::NoSession {
            host: Some("quilt.test".to_string()),
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
    /// answers "what may I choose to do", so it is present in every state —
    /// including the ones that publish, whose caret offers `Review before
    /// publishing…` instead.
    #[wasm_bindgen_test]
    fn create_new_revision_is_in_the_menu_in_every_state() {
        for state in [
            kit::PackageState::PendingCommit,
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
            host: "quilt.test".to_string(),
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
        // The options, not the dialog's text: the dialog names the refused role
        // in its sentence, and the rule here is only that it is not offered.
        let options = dialog.query_selector_all("option").unwrap();
        let offered: Vec<String> = (0..options.length())
            .map(|i| options.item(i).unwrap().text_content().unwrap_or_default())
            .collect();
        assert_eq!(
            offered,
            vec!["admin".to_string()],
            "the alternatives are the options, and the refused role is not one of them; \
             markup was {}",
            dialog.inner_html()
        );
    }

    /// Open the role dialog over a denial of `role`, with one other role held.
    async fn open_role_dialog(role: Option<&str>) -> web_sys::Element {
        let mut d = data(kit::PackageState::RoleDenied {
            role: role.map(str::to_string),
        });
        d.role_switch = Some(commands::RoleSwitch {
            host: "quilt.test".to_string(),
            alternatives: vec!["admin".to_string()],
        });
        let el = mount_header(d);
        button(&el, "Switch role").click();
        // The dialog opens from an effect, on the next tick.
        leptos::task::tick().await;
        el
    }

    /// The dialog names the refusal before its select: the reader is choosing
    /// what replaces a role, so the dialog says which one failed. The chip
    /// stays one state label, and does not.
    #[wasm_bindgen_test]
    async fn the_role_dialog_names_the_refused_role() {
        let el = open_role_dialog(Some("analyst")).await;

        let sentence = element_saying(&el, "You're using analyst, which can't read this package.");
        assert!(
            sentence.closest("dialog[open]").unwrap().is_some(),
            "the sentence is in the open dialog; markup was {}",
            el.inner_html()
        );
        let strong = sentence
            .query_selector("strong")
            .unwrap()
            .unwrap_or_else(|| panic!("the role, set apart; markup was {}", sentence.inner_html()));
        assert_eq!(strong.text_content().unwrap_or_default(), "analyst");
        element_saying(&el, "No access");
    }

    /// A denial that names no role still says why the dialog is open, without
    /// inventing a name.
    #[wasm_bindgen_test]
    async fn a_denial_naming_no_role_says_your_current_role() {
        let el = open_role_dialog(None).await;

        let sentence = element_saying(&el, "Your current role can't read this package.");
        assert!(
            sentence.closest("dialog[open]").unwrap().is_some(),
            "the sentence is in the open dialog; markup was {}",
            el.inner_html()
        );
        assert!(
            sentence.query_selector("strong").unwrap().is_none(),
            "no name to set apart; markup was {}",
            sentence.inner_html()
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
            host: "quilt.test".to_string(),
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
            words.contains(
                "Deletes this package's working files, including edits and commits that were \
                 never pushed."
            ),
            "the consequence names what is lost: {words}"
        );
        assert!(
            !words.contains("keeps committed content"),
            "no promise that unpushed commits survive: {words}"
        );
        assert_eq!(
            footer_labels(&el),
            vec!["Cancel".to_string(), "Remove".to_string()]
        );
    }

    /// Remove's confirmation offers to delete the downloaded files, checked
    /// each time it opens: unchecking it once does not carry to the next.
    #[wasm_bindgen_test]
    async fn remove_offers_to_delete_the_downloaded_files_checked_each_time() {
        let el = mount_header(data(kit::PackageState::Latest));
        let checkbox = || -> web_sys::HtmlInputElement {
            el.query_selector("dialog[open] input[type=checkbox]")
                .unwrap()
                .expect("the checkbox")
                .unchecked_into()
        };
        open_menu(&el);
        menu_item(&el, "Remove").click();
        sleep_ms(20).await;
        let words = el
            .query_selector("dialog[open]")
            .unwrap()
            .expect("the confirmation")
            .text_content()
            .unwrap_or_default();
        assert!(
            words.contains("Also delete downloaded files from disk"),
            "{words}"
        );
        assert!(checkbox().checked(), "checked when it opens");

        checkbox().click();
        sleep_ms(20).await;
        assert!(!checkbox().checked());
        let footer = el.query_selector_all("dialog[open] button").unwrap();
        (0..footer.length())
            .map(|i| {
                footer
                    .item(i)
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlElement>()
            })
            .find(|b| b.text_content().unwrap_or_default().trim() == "Cancel")
            .expect("Cancel")
            .click();
        sleep_ms(20).await;

        open_menu(&el);
        menu_item(&el, "Remove").click();
        sleep_ms(20).await;
        assert!(checkbox().checked(), "checked again on the next opening");
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

    /// A re-read rebuilds the header while a dialog's command runs, and the
    /// dialog survives it on the page's flag. Mounted in that moment: the flag
    /// open and the page's signal held. The new dialog has no in-flight state
    /// of its own, so the page's is what keeps it from a second command.
    #[wasm_bindgen_test]
    async fn a_dialog_rebuilt_mid_command_cannot_run_it_again() {
        let w = Wiring {
            busy: RwSignal::new(true),
            ..Wiring::new()
        };
        w.dialogs.remove.set(true);
        let el = mount_with(data(kit::PackageState::Latest), w);
        // The dialog opens from an effect, on the next tick.
        leptos::task::tick().await;

        let buttons = el.query_selector_all("dialog[open] button").unwrap();
        let verb: web_sys::HtmlButtonElement = (0..buttons.length())
            .map(|i| {
                buttons
                    .item(i)
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlButtonElement>()
            })
            .find(|b| b.text_content().unwrap_or_default().trim() == "Remove")
            .unwrap_or_else(|| panic!("the confirmation; markup was {}", el.inner_html()));
        assert!(verb.disabled(), "sealed while the first one runs");
        assert_eq!(verb.get_attribute("aria-busy").as_deref(), Some("true"));
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

    /// Nothing else on the header confirms. A third `ConfirmDialog` here would be
    /// a tone rule nobody decided; the pane's `Replace mine` is the
    /// page's one other, decided in `replace-confirmation`.
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
            "quilt+s3://team-bucket#package=team/dataset&catalog=quilt.test"
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

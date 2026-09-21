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
//! # What is wired, and what is not
//!
//! Every command acts except *Change bucket*, which needs the set-remote prompt
//! rebuilt on this kit and is disabled saying so.
//!
//! A denial still offers nothing, though it is
//! [ruled](/changes/.archive/2026/09/quiltsync-package-header.md#denial-action)
//! to offer *Switch role*: doing that honestly means knowing the reader holds
//! another role, which is a `RoleCache` round trip this page's payload does not
//! make. The kit has no `SwitchRole` verb yet either, so the row is empty rather
//! than disabled — there is nothing to disable.
//!
//! # Disabled rather than inert
//!
//! A control that accepts a click and answers with silence is worse than one that
//! shows it is unavailable, so anything this page cannot do is disabled and
//! states its reason.
//!
//! *Undo last revision* states three different ones, because the engine refuses
//! three ways and the reader can act on the difference: nothing committed yet,
//! a first revision with nothing behind it, or a package with a remote, where
//! pushing has consumed the commit chain.

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
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

stylance::import_crate_style!(style, "src/pages/installed_package_v2/header.module.scss");

/// What a command reported, on its way to the page's band.
#[derive(Clone)]
pub(super) struct Outcome {
    pub ok: bool,
    pub message: String,
}

/// The page's half of a command: where it reports, what it blocks while it
/// runs, and what to re-read when it is done.
///
/// One value because the three always travel together — every control that can
/// start a command needs all of them, and none of them means anything alone.
#[derive(Clone, Copy)]
pub(super) struct Wiring {
    pub busy: RwSignal<bool>,
    pub outcome: RwSignal<Option<Outcome>>,
    pub reload: Trigger,
}

/// Why the two unbuilt commands are unavailable.
/// While another command is running. Every control takes it, so a second
/// cannot start on top of the first.
const BUSY: &str = "Something else is running";
const NO_PROMPT: &str = "The bucket picker is not on this page yet";

/// Run a command, report what it said, and refetch when it changed something.
///
/// `busy` is raised for the whole call so a second command cannot start on top
/// of the first — every control reads it.
///
/// An **empty** success message is not a missing one: a command that reported
/// through the notification stack returns one deliberately, so the page does not
/// say the same thing a second time from behind the toast layer. `package_pull`
/// is the case.
fn run(
    busy: RwSignal<bool>,
    outcome: RwSignal<Option<Outcome>>,
    after: Option<Trigger>,
    task: impl std::future::Future<Output = Result<String, String>> + 'static,
) {
    busy.set(true);
    leptos::task::spawn_local(async move {
        let answer = task.await;
        busy.set(false);
        match answer {
            Ok(message) => {
                if !message.is_empty() {
                    outcome.set(Some(Outcome { ok: true, message }));
                }
                if let Some(reload) = after {
                    reload.notify();
                }
            }
            Err(message) => outcome.set(Some(Outcome { ok: false, message })),
        }
    });
}

/// Why undo is unavailable, when it is — the engine's three refusals, told
/// apart so the reader knows which one they are looking at.
///
/// `None` means it is available. The fourth refusal, a working tree holding
/// uncommitted edits, is deliberately not here: it is transient, and the engine
/// states it when the command runs rather than the menu hiding it.
fn undo_blocked(data: &commands::PackageHeaderData) -> Option<&'static str> {
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

/// The package-level commands. Fixed across states on purpose: the menu is where
/// everything that is *not* the one primary action lives, so it does not change
/// shape as the state does. What varies is what a command can be offered *for* —
/// a catalog link needs a catalog, and an undo needs something to undo.
fn menu(
    data: &commands::PackageHeaderData,
    w: Wiring,
    goto: RwSignal<Option<String>>,
) -> Vec<MenuAction> {
    let Wiring {
        busy,
        outcome,
        reload,
    } = w;
    let ns = data.namespace.clone();
    let commit_to = crate::routes::commit_href(&ns);
    let mut actions = vec![MenuAction {
        label: "Create new revision".to_string(),
        tone: ActionTone::Default,
        disabled: busy.get().then(|| BUSY.to_string()),
        on_select: Callback::new(move |()| goto.set(Some(commit_to.clone()))),
        separated: false,
    }];

    // Dropped rather than disabled: a package with no catalog has no catalog
    // page, so there is no reason to state and nothing the reader could do.
    if let Some(url) = data.uri.as_ref().and_then(util::catalog_url) {
        actions.push(MenuAction {
            label: "Open in catalog".to_string(),
            tone: ActionTone::Default,
            disabled: busy.get().then(|| BUSY.to_string()),
            on_select: Callback::new(move |()| {
                let url = url.clone();
                leptos::task::spawn_local(async move {
                    let _ = commands::open_in_web_browser(url).await;
                });
            }),
            separated: false,
        });
    }

    // A pushed package is pinned to its push history, so the bucket can be shown
    // and not changed. Same command, honest label — v1's toolbar makes the same
    // swap. Disabled either way until this page has a bucket picker.
    actions.push(MenuAction {
        label: if data.remote_locked {
            "Show remote".to_string()
        } else {
            "Change bucket".to_string()
        },
        tone: ActionTone::Default,
        disabled: Some(NO_PROMPT.to_string()),
        on_select: Callback::new(|()| ()),
        separated: false,
    });

    let ns_undo = ns.clone();
    actions.push(MenuAction {
        label: "Undo last revision".to_string(),
        tone: ActionTone::Danger,
        disabled: undo_blocked(data)
            .map(ToString::to_string)
            .or_else(|| busy.get().then(|| BUSY.to_string())),
        on_select: Callback::new(move |()| {
            let ns = ns_undo.to_string();
            run(busy, outcome, Some(reload), async move {
                commands::undo_commit(ns).await
            });
        }),
        separated: true,
    });

    let ns_remove = ns.clone();
    let uri_remove = data.uri.clone();
    actions.push(MenuAction {
        label: "Remove".to_string(),
        tone: ActionTone::Danger,
        disabled: busy.get().then(|| BUSY.to_string()),
        on_select: Callback::new(move |()| {
            let ns = ns_remove.to_string();
            let uri = uri_remove.clone();
            busy.set(true);
            leptos::task::spawn_local(async move {
                let answer = commands::package_uninstall(ns, uri).await;
                busy.set(false);
                match answer {
                    // Home, not a refetch: the package this page is about is
                    // gone, so re-reading it would ask for something that no
                    // longer exists.
                    Ok(_) => goto.set(Some("/".to_string())),
                    Err(message) => outcome.set(Some(Outcome { ok: false, message })),
                }
            });
        }),
        separated: false,
    });

    actions
}

/// The state's own action, as the row draws it.
///
/// Both shapes live here because they are one slot: the publishing states get a
/// split button whose caret holds `Create new revision`, and the rest get their
/// verb plainly. `Latest` and a denial get neither, and the row keeps its height
/// either way.
fn primary_action(
    data: &commands::PackageHeaderData,
    action: Option<PackageAction>,
    w: Wiring,
    goto: RwSignal<Option<String>>,
    publish_choice: RwSignal<usize>,
) -> AnyView {
    let Wiring {
        busy,
        outcome,
        reload,
    } = w;
    let ns = data.namespace.clone();
    let uri = data.uri.clone();
    let resolve_to = crate::routes::merge_href(&ns);
    let publish_to = crate::routes::commit_href(&ns);
    let revision_to = crate::routes::commit_href(&ns);
    // The deployment to sign in to, when the state names one. `None` for a bare
    // bucket on ambient credentials, which is why the kit offers no action there.
    let sign_in_to = match &data.state {
        kit::PackageState::NoSession { host: Some(host) }
        | kit::PackageState::SignInExpired { host: Some(host) } => {
            Some(crate::routes::sign_in_href(host))
        }
        _ => None,
    };
    // The one primary this page cannot run: choosing a bucket opens the prompt
    // that has not been rebuilt here yet.
    let blocked = matches!(action, Some(PackageAction::ChooseS3Bucket));

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
            run(busy, outcome, Some(reload), async move {
                commands::package_pull(ns, uri).await
            });
        }
        Some(PackageAction::Resolve) => goto.set(Some(resolve_to.clone())),
        Some(PackageAction::SignIn) => goto.set(sign_in_to.clone()),
        // `Publish` returned above; the bucket picker is gated; a denial arrives
        // as `None` until the role query lands.
        Some(PackageAction::Publish | PackageAction::ChooseS3Bucket) | None => (),
    };

    action.map_or_else(
        || ().into_any(),
        |action| {
            view! {
                <span class=style::action_slot data-primary-action>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=Signal::derive(move || busy.get() || blocked)
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

/// The header, for one package.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
pub fn PageHeader(data: commands::PackageHeaderData, w: Wiring) -> impl IntoView {
    let Wiring { busy, outcome, .. } = w;
    let rendered = render(&data.state, kit::Site::PageHeader);
    let action = rendered.action;
    let namespace = data.namespace.to_string();
    let publish_choice = RwSignal::new(0_usize);

    // Every navigation this header makes goes through one signal, because a
    // `Callback` must be `Send + Sync` and `use_navigate`'s closure is neither.
    // One effect performs them, which also keeps the router call in one place.
    let goto: RwSignal<Option<String>> = RwSignal::new(None);
    let navigate = use_navigate();
    Effect::new(move |_| {
        if let Some(target) = goto.get() {
            navigate(&target, NavigateOptions::default());
            goto.set(None);
        }
    });
    let actions = menu(&data, w, goto);

    let ns_folder = data.namespace.clone();
    let uri_folder = data.uri.clone();
    let on_open_folder = move |_| {
        let ns = ns_folder.to_string();
        let uri = uri_folder.clone();
        run(busy, outcome, None, async move {
            commands::open_in_file_browser(ns, uri).await
        });
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
                    {primary_action(&data, action, w, goto, publish_choice)}
                    <Button disabled=Signal::derive(move || busy.get()) on_click=on_open_folder>
                        "Open folder"
                    </Button>
                    <ActionMenu aria_label="More actions for this package" actions=actions />
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
    use leptos_router::components::Router;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The split button's preference menu, which holds choices and not commands.
    const SPLIT_CHOICES: &str = "[aria-label='Change what this button does']";

    /// Mount a header the way the page does — inside a `Router`, because it
    /// asks for a navigator and `use_navigate` panics without one.
    ///
    /// The wiring is supplied and then ignored: a mounted header needs
    /// somewhere to put a result and something to read for busy, and no test
    /// here presses a command.
    fn mount_header(data: commands::PackageHeaderData) -> web_sys::Element {
        mount(move || {
            let busy = RwSignal::new(false);
            let outcome: RwSignal<Option<Outcome>> = RwSignal::new(None);
            let reload = Trigger::new();
            view! {
                <Router>
                    <PageHeader data=data.clone() w=Wiring { busy, outcome, reload } />
                </Router>
            }
        })
    }

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

    /// What is still unavailable, and why each one says a different thing.
    ///
    /// Only *Change bucket* is disabled now, and its reason is the surface it
    /// needs rather than a blanket "not yet" — the distinction matters because
    /// everything around it works, so a reader has to be able to tell a missing
    /// prompt from a broken button.
    #[wasm_bindgen_test]
    fn change_bucket_is_the_one_command_still_waiting_on_a_surface() {
        // Undo available, so the only thing left disabled is the one waiting on
        // a surface. With the default fixture undo is disabled too, for a
        // reason of its own, and this test would not be saying what it means.
        let mut d = data(kit::PackageState::Behind);
        d.has_local_commit = true;
        d.commit_has_parent = true;
        let el = mount_header(d);

        let buttons = el.query_selector_all("button").unwrap();
        let mut disabled = Vec::new();
        for i in 0..buttons.length() {
            let b: web_sys::Element = buttons.item(i).unwrap().unchecked_into();
            if b.closest(SPLIT_CHOICES).unwrap().is_some() {
                continue;
            }
            if b.has_attribute("disabled") {
                disabled.push(b.text_content().unwrap_or_default().trim().to_string());
            }
        }
        assert_eq!(disabled.len(), 1, "markup was {}", el.inner_html());
        assert!(disabled[0].starts_with("Change bucket"), "got {disabled:?}");
        assert!(
            disabled[0].contains("bucket picker"),
            "and it states the surface it is waiting on, not a blanket reason: {disabled:?}"
        );
    }

    /// Undo tells its three refusals apart, because the reader can act on the
    /// difference: commit something, or accept that a pushed package has no
    /// chain left. A single reason would make the first look like the third.
    ///
    /// The fourth refusal — a dirty tree — is deliberately absent: it is
    /// transient, and the engine states it when the command runs.
    #[wasm_bindgen_test]
    fn undo_says_which_of_the_three_refusals_it_hit() {
        let cases = [
            (false, false, None, "Nothing has been committed yet"),
            (true, false, None, "first revision"),
            (
                true,
                true,
                Some("quilt+s3://team-bucket#package=team/dataset"),
                "before the first push",
            ),
        ];
        for (has_commit, has_parent, uri, expected) in cases {
            let mut d = data(kit::PackageState::Latest);
            d.has_local_commit = has_commit;
            d.commit_has_parent = has_parent;
            d.uri = uri.map(|u| u.parse().expect("a package uri"));
            let el = mount_header(d);

            let items = el.query_selector_all("button").unwrap();
            let mut undo = None;
            for i in 0..items.length() {
                let b: web_sys::Element = items.item(i).unwrap().unchecked_into();
                let text = b.text_content().unwrap_or_default();
                if text.starts_with("Undo last revision") {
                    undo = Some((b.has_attribute("disabled"), text));
                }
            }
            let (is_disabled, text) = undo.expect("the undo item");
            assert!(is_disabled, "{expected}: should be disabled");
            assert!(text.contains(expected), "wanted {expected:?}, got {text:?}");
        }
    }

    /// And it is offered when none of the three bites. Without this the test
    /// above passes against an item that is always disabled.
    #[wasm_bindgen_test]
    fn undo_is_offered_when_the_chain_reaches_back_and_there_is_no_remote() {
        let mut d = data(kit::PackageState::Latest);
        d.has_local_commit = true;
        d.commit_has_parent = true;
        d.uri = None;
        let el = mount_header(d);

        let items = el.query_selector_all("button").unwrap();
        let mut found = false;
        for i in 0..items.length() {
            let b: web_sys::Element = items.item(i).unwrap().unchecked_into();
            if b.text_content()
                .unwrap_or_default()
                .starts_with("Undo last revision")
            {
                assert!(
                    !b.has_attribute("disabled"),
                    "markup was {}",
                    el.inner_html()
                );
                found = true;
            }
        }
        assert!(found, "the undo item");
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
            let el = mount_header(data(state.clone()));
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

//! The set-remote dialog: *Change bucket* over host, bucket and workflow, and
//! *Show remote*, the same dialog with nothing to submit.
//!
//! # The fields are the gallery's; the machinery is v1's
//!
//! The markup is `gallery/forms.rs`'s `BucketForm`, which is where the shape
//! was settled. What makes it real — validation, the debounced workflow read,
//! and the self-keying that ties a submitted workflow to the bucket on screen —
//! is carried over from `components/set_remote_popup.rs`. What v1 hand-wrote
//! around it (the overlay, Enter and Escape, the in-flight flag, the refusal)
//! is `FormDialog`'s now, which is the reason the form moved.
//!
//! # Two shapes, one command
//!
//! A pushed package is pinned to its push history, so `remote_locked` selects
//! the read-only shape: every control disabled and `submit: None`, which draws
//! one `Close` and no form. The state's *Choose S3 bucket* opens the editable
//! shape, same as the menu's *Change bucket*.

use std::time::Duration;

use leptos::prelude::*;
use leptos::reactive::computed::suspense::SuspenseContext;
use leptos::reactive::signal::ArcRwSignal;

use crate::commands;
use crate::commands::{CommitWorkflows, WorkflowIntent};
use crate::components::build_workflow_view;
use crate::components::workflow_select::{WorkflowView, WorkflowViewKind};
use crate::kit::{BannerVariant, FormControl, FormDialog, Naming, Select, Submit, TextInput};
use crate::util;

use super::{Outcome, Wiring};

const HOST_INVALID: &str = "Enter a valid hostname";
const BUCKET_MISSING: &str = "Enter an S3 bucket name";
/// v1's words for the workflow read in flight.
const LOADING: &str = "Loading workflows…";

/// Whether the typed pair can be submitted, and what is wrong when it cannot.
/// Pure, so the rule is testable without a dialog.
pub(super) fn field_errors(
    host: &str,
    bucket: &str,
) -> (Option<&'static str>, Option<&'static str>) {
    let host_error = (host.is_empty() || !util::is_valid_hostname(host)).then_some(HOST_INVALID);
    let bucket_error = bucket.is_empty().then_some(BUCKET_MISSING);
    (host_error, bucket_error)
}

/// The options a reader can pick, as the kit's `Select` draws them. It has no
/// disabled option, so an option the view marks disabled is left out.
fn choosable(view: &WorkflowView) -> Vec<String> {
    match view.kind {
        WorkflowViewKind::Available { .. } => view
            .options
            .iter()
            .filter(|option| !option.disabled)
            .map(|option| option.label.clone())
            .collect(),
        // One disabled entry, which is the whole answer: there is no choice.
        _ => view
            .options
            .iter()
            .map(|option| option.label.clone())
            .collect(),
    }
}

/// The label that starts selected: the view's own preselection, or the first
/// choosable option when the preselection is one the reader may not pick.
fn initial_label(view: &WorkflowView) -> String {
    let labels = choosable(view);
    view.options
        .get(view.initial)
        .map(|option| option.label.clone())
        .filter(|label| labels.contains(label))
        .or_else(|| labels.first().cloned())
        .unwrap_or_default()
}

/// The set-remote dialog, in both of its shapes.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads it from there"
)]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub(super) fn BucketDialog(
    open: RwSignal<bool>,
    data: commands::PackageHeaderData,
    w: Wiring,
) -> impl IntoView {
    // The workflow read must not re-trigger the page's `<Suspense>`; see
    // `components/set_remote_popup.rs` for the full argument.
    #[allow(
        clippy::default_trait_access,
        reason = "SuspenseContext.tasks is leptos's internal slotmap::SlotMap; naming it would add a `slotmap` dependency for a nitpick"
    )]
    provide_context(SuspenseContext {
        tasks: ArcRwSignal::new(Default::default()),
    });

    // `busy` is unused on purpose: `FormDialog` seals itself while its submit
    // runs, and the page's signal is for commands no dialog is holding.
    let Wiring {
        busy: _,
        outcome,
        reload,
    } = w;
    let locked = data.remote_locked;
    let ns = data.namespace.to_string();
    let current_host = data
        .uri
        .as_ref()
        .and_then(util::host_str)
        .unwrap_or_default();
    let current_bucket = data
        .uri
        .as_ref()
        .and_then(util::bucket_str)
        .unwrap_or_default();

    let origin = RwSignal::new(current_host.clone());
    let bucket = RwSignal::new(current_bucket.clone());
    let host_invalid: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let bucket_invalid: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let selected_workflow = RwSignal::new(String::new());

    // Each opening starts from the package's remote, not from what was typed
    // into the last one and cancelled. Only a real opening: the first run lands
    // a tick after mount, when the fields already hold these values.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if is_open && was_open == Some(false) {
            origin.set(current_host.clone());
            bucket.set(current_bucket.clone());
            host_invalid.set(None);
            bucket_invalid.set(None);
        }
        is_open
    });
    // A field's message is about the value it was raised for; typing retracts it.
    Effect::new(move |_| {
        origin.track();
        host_invalid.set(None);
    });
    Effect::new(move |_| {
        bucket.track();
        bucket_invalid.set(None);
    });

    // `Some` only when the dialog is open and the pair could be submitted, so a
    // closed dialog reads nothing from S3.
    let valid_target = Memo::new(move |_| {
        let host = origin.get().trim().to_string();
        let name = bucket.get().trim().to_string();
        (open.get() && field_errors(&host, &name) == (None, None)).then_some((host, name))
    });

    // One fetch per settled bucket name rather than per keystroke, which would
    // flash the Unavailable notice and hit S3 for every intermediate name.
    let debounced_target = RwSignal::new(valid_target.get_untracked());
    let debounce_timer: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    Effect::new(move |_| {
        let target = valid_target.get();
        if let Some(handle) = debounce_timer.get_value() {
            handle.clear();
        }
        if target.is_none() {
            debounced_target.set(None);
        } else if let Ok(handle) = set_timeout_with_handle(
            move || debounced_target.set(target),
            Duration::from_millis(400),
        ) {
            debounce_timer.set_value(Some(handle));
        }
    });
    on_cleanup(move || {
        if let Some(Some(handle)) = debounce_timer.try_get_value() {
            handle.clear();
        }
    });

    let workflows = LocalResource::new(move || {
        let target = debounced_target.get();
        async move {
            let (host, name) = target?;
            let answer = commands::get_bucket_workflows(host.clone(), name.clone()).await;
            Some(((host, name), answer))
        }
    });

    // Self-keyed: a result for a target that is no longer the typed one is
    // ignored, which covers the debounce window and an in-flight refetch alike.
    // A first remote has no previous revision, so the bucket default is
    // preselected.
    let wf_view = Memo::new(move |_| {
        workflows.get().flatten().and_then(|(target, answer)| {
            (Some(target) == valid_target.get()).then(|| match answer {
                Ok(config) => build_workflow_view(&config, None),
                Err(_) => build_workflow_view(&CommitWorkflows::Unavailable, None),
            })
        })
    });

    // Every not-yet-current window, because `wf_view` is self-keyed.
    let workflow_loading =
        Memo::new(move |_| valid_target.get().is_some() && wf_view.get().is_none());

    // Display and submit agree: a new view restarts at its preselection.
    Effect::new(move |_| {
        selected_workflow.set(match wf_view.get() {
            Some(view) => initial_label(&view),
            None if workflow_loading.get() => LOADING.to_string(),
            // No bucket to read yet, so nothing to choose.
            None => String::new(),
        });
    });

    let submit = (!locked).then(|| {
        // `Submit::new` takes an `Fn`, so the closure clones what it owns per
        // call and the `async move` block captures the clone. Every signal it
        // touches is `Copy` and needs no clone.
        Submit::new("Save", move || {
            let ns = ns.clone();
            async move {
                let host = origin.get_untracked().trim().to_string();
                let name = bucket.get_untracked().trim().to_string();
                let (host_error, bucket_error) = field_errors(&host, &name);
                // Per-field, so it draws under the field it is about rather than
                // as the dialog's banner, which is for the backend refusing the
                // whole submission.
                host_invalid.set(host_error);
                bucket_invalid.set(bucket_error);
                if host_error.is_some() || bucket_error.is_some() {
                    return Err("Fix the highlighted fields.".to_string());
                }
                // The submitted workflow must belong to the bucket on screen.
                if workflow_loading.get_untracked() {
                    return Err(
                        "This bucket's workflows are still loading. Try again in a moment."
                            .to_string(),
                    );
                }
                // The selected option's own intent, passed straight through, and
                // `BucketDefault` when the config is not loaded — never
                // `Named("")`. v1's rule.
                let workflow = wf_view
                    .get_untracked()
                    .and_then(|view| {
                        let label = selected_workflow.get_untracked();
                        view.options
                            .into_iter()
                            .find(|option| option.label == label)
                            .map(|option| option.intent)
                    })
                    .unwrap_or(WorkflowIntent::BucketDefault);
                let response = commands::set_remote(ns.clone(), host, name, workflow).await?;
                // The remote is set either way. A workflow that could not be
                // resolved is the one partial success the band carries: the
                // package will publish ungoverned, and the reader should learn
                // that now rather than at push time.
                if let Some(reason) = response.resolution_warning {
                    outcome.set(Some(Outcome {
                        namespace: ns,
                        variant: BannerVariant::Warning,
                        lead: "The remote is set, but the bucket's default workflow could not \
                               be resolved, so this package will publish without one."
                            .to_string(),
                        detail: Some(reason),
                    }));
                }
                reload.notify();
                Ok(())
            }
        })
    });

    // The config's own complaint, under the field it is about. Only a malformed
    // config has one: the other states are answers, not problems.
    let workflow_error = Signal::derive(move || match wf_view.get().map(|view| view.kind) {
        Some(WorkflowViewKind::Invalid { reason }) => Some(format!(
            "This bucket's workflow configuration is invalid ({reason}). Publishing to this \
             bucket will fail until it is fixed."
        )),
        _ => None,
    });

    let title = if locked {
        "Show remote"
    } else {
        "Change bucket"
    };

    // Built once, in whichever shape the package is in: the kit's `FormDialog`
    // takes its `submit` as a value, not an `Option`.
    let fields = move || {
        view! {
            // Host and Bucket are named concretely on purpose: this is where the
            // reader chooses an endpoint — see `gallery/forms.rs`.
            <FormControl
                label="Host"
                caption=if locked {
                    "Where this package is published."
                } else {
                    "Where this package is published. From your accounts."
                }
                error=Signal::derive(move || host_invalid.get().map(str::to_string))
                control=move |id| {
                    view! {
                        <TextInput
                            id=id
                            value=origin
                            placeholder="open.quiltdata.com"
                            disabled=locked
                            invalid=Signal::derive(move || host_invalid.get().is_some())
                        />
                    }
                        .into_any()
                }
            />
            {if locked {
                view! {
                    <FormControl
                        label="Bucket"
                        caption="Fixed by this package's push history."
                        control=move |id| {
                            view! { <TextInput id=id value=bucket disabled=true /> }.into_any()
                        }
                    />
                }
                    .into_any()
            } else {
                view! {
                    <FormControl
                        label="Bucket"
                        required=true
                        error=Signal::derive(move || bucket_invalid.get().map(str::to_string))
                        control=move |id| {
                            view! {
                                <TextInput
                                    id=id
                                    value=bucket
                                    placeholder="my-s3-bucket"
                                    invalid=Signal::derive(move || bucket_invalid.get().is_some())
                                />
                            }
                                .into_any()
                        }
                    />
                }
                    .into_any()
            }}
            // Read from the bucket once it is known, so it is last. Redrawn as the
            // read settles, because the kit's `Select` takes its options once.
            <FormControl
                label="Workflow"
                caption="Rules the bucket applies when you publish."
                error=workflow_error
                control=move |id| {
                    view! {
                        {move || {
                            let (options, choosing) = match wf_view.get() {
                                Some(view) => {
                                    let choosing = matches!(
                                        view.kind,
                                        WorkflowViewKind::Available { .. }
                                    );
                                    (choosable(&view), choosing)
                                }
                                None if workflow_loading.get() => {
                                    (vec![LOADING.to_string()], false)
                                }
                                None => (Vec::new(), false),
                            };
                            view! {
                                <Select
                                    naming=Naming::FormControl(id.clone())
                                    options=options
                                    selected=selected_workflow
                                    disabled=locked || !choosing
                                />
                            }
                        }}
                    }
                        .into_any()
                }
            />
        }
    };

    match submit {
        Some(submit) => view! {
            <FormDialog open=open title=title submit=submit>{fields()}</FormDialog>
        }
        .into_any(),
        None => view! { <FormDialog open=open title=title>{fields()}</FormDialog> }.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn data() -> commands::PackageHeaderData {
        commands::PackageHeaderData {
            namespace: "team/dataset".try_into().unwrap(),
            uri: None,
            state: kit::PackageState::NoRemote,
            remote_locked: false,
            has_local_commit: false,
            commit_has_parent: false,
            role_switch: None,
        }
    }

    fn mount_dialog(data: commands::PackageHeaderData) -> web_sys::Element {
        mount(move || {
            let w = Wiring {
                busy: RwSignal::new(false),
                outcome: RwSignal::new(None),
                reload: Trigger::new(),
            };
            view! { <BucketDialog open=RwSignal::new(true) data=data w=w /> }
        })
    }

    /// What the footer offers, in order. The fields hold no buttons, so every
    /// button in the dialog is the footer's.
    fn footer_labels(el: &web_sys::Element) -> Vec<String> {
        let all = el.query_selector_all("dialog button").unwrap();
        (0..all.length())
            .map(|i| {
                all.item(i)
                    .unwrap()
                    .text_content()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// The control a field's label names, found through its `for`. By prefix,
    /// because a required field's label carries `(required)` after its name.
    fn field(el: &web_sys::Element, label: &str) -> web_sys::Element {
        let labels = el.query_selector_all("label").unwrap();
        let target = (0..labels.length())
            .map(|i| labels.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|l| {
                l.text_content()
                    .unwrap_or_default()
                    .trim()
                    .starts_with(label)
            })
            .and_then(|l| l.get_attribute("for"))
            .unwrap_or_else(|| {
                panic!(
                    "no field is labelled {label:?}; markup was {}",
                    el.inner_html()
                )
            });
        el.query_selector(&format!("#{target}"))
            .unwrap()
            .unwrap_or_else(|| panic!("the label for {label:?} points at nothing"))
    }

    fn type_into(el: &web_sys::Element, label: &str, value: &str) {
        let input: web_sys::HtmlInputElement = field(el, label).unchecked_into();
        input.set_value(value);
        let event = web_sys::InputEvent::new("input").unwrap();
        input.dispatch_event(&event).unwrap();
    }

    /// v1's rule, carried over: a host must be a hostname and a bucket must be
    /// something. Both messages name the field's own problem, because the dialog
    /// banner is for the backend refusing the whole submission.
    #[test]
    fn the_form_states_what_is_wrong_per_field() {
        assert_eq!(
            field_errors("", ""),
            (
                Some("Enter a valid hostname"),
                Some("Enter an S3 bucket name")
            )
        );
        assert_eq!(
            field_errors("not a host", "b"),
            (Some("Enter a valid hostname"), None)
        );
        assert_eq!(
            field_errors("open.quiltdata.com", ""),
            (None, Some("Enter an S3 bucket name"))
        );
        assert_eq!(
            field_errors("open.quiltdata.com", "my-bucket"),
            (None, None)
        );
    }

    /// The read-only shape is what `remote_locked` selects, and it is not a
    /// spare variant: a package pinned to its push history has a remote to show
    /// and not to change. `FormDialog`'s `submit: None` draws one `Close` and no
    /// form at all.
    #[wasm_bindgen_test]
    fn a_pushed_package_shows_its_remote_and_offers_no_save() {
        let mut d = data();
        d.remote_locked = true;
        let el = mount_dialog(d);

        assert!(
            el.query_selector("form").unwrap().is_none(),
            "nothing to submit"
        );
        let labels = footer_labels(&el);
        assert_eq!(labels, vec!["Close".to_string()], "one way out");
    }

    /// The editable shape carries all three fields and the verb.
    #[wasm_bindgen_test]
    fn the_editable_shape_offers_host_bucket_workflow_and_save() {
        let el = mount_dialog(data());
        for label in ["Host", "Bucket", "Workflow"] {
            field(&el, label);
        }
        assert_eq!(
            footer_labels(&el),
            vec!["Cancel".to_string(), "Save".to_string()]
        );
    }

    /// There is no Tauri bridge under the runner, so the workflow fetch fails —
    /// which is `CommitWorkflows::Unavailable`, the state v1 degrades to, and the
    /// control says so rather than showing an empty list. Submit still carries
    /// `BucketDefault`, so the real default re-resolves on the backend.
    #[wasm_bindgen_test]
    async fn a_failed_workflow_fetch_degrades_to_the_bucket_default() {
        let el = mount_dialog(data());
        type_into(&el, "Host", "open.quiltdata.com");
        type_into(&el, "Bucket", "my-bucket");
        sleep_ms(600).await;

        element_saying(&el, "Bucket default");
    }
}

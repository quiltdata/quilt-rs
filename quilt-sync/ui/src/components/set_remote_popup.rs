use std::time::Duration;

use leptos::prelude::*;
use leptos::reactive::computed::suspense::SuspenseContext;
use leptos::reactive::signal::ArcRwSignal;
use wasm_bindgen::JsCast;

use crate::commands;
use crate::commands::{CommitWorkflows, WorkflowIntent};
use crate::components::Notification;
use crate::components::buttons;
use crate::components::{WorkflowSection, build_workflow_view};
use crate::util::is_valid_hostname;

#[derive(Clone, Debug)]
pub struct SetRemotePopupData {
    pub namespace: String,
    pub current_host: Option<String>,
    pub current_bucket: Option<String>,
    pub has_local_commit: bool,
}

/// The popup's title. A published package keeps its bucket, so the popup
/// only adds or changes its catalog.
fn title(published: bool, has_host: bool) -> &'static str {
    match (published, has_host) {
        (false, _) => "Set remote",
        (true, false) => "Add catalog",
        (true, true) => "Change catalog",
    }
}

/// Whether each typed field is wrong: `(host, bucket)`. The host is optional,
/// and without it the bucket is reached on this computer's AWS credentials.
fn field_errors(host: &str, bucket: &str) -> (bool, bool) {
    (
        !host.is_empty() && !is_valid_hostname(host),
        bucket.is_empty(),
    )
}

#[component]
#[allow(clippy::needless_pass_by_value)]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub fn SetRemotePopup(
    namespace: String,
    current_host: Option<String>,
    current_bucket: Option<String>,
    /// The package is published, so its bucket is fixed: the bucket field is
    /// locked, there is no workflow to choose, and Save changes only the
    /// catalog (see `InstalledPackage::change_catalog` in quilt-rs).
    #[prop(optional)]
    published: bool,
    /// True when the package has a local commit. Setting a remote only
    /// re-commits (creating a new revision) when there is a commit to
    /// re-commit, so the "creates a new revision" notice shows only then.
    #[prop(optional)]
    has_local_commit: bool,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    // Isolate this popup's workflow fetch from the packages-list page's
    // <Suspense>. Reading a `LocalResource` registers the read with the nearest
    // `SuspenseContext` (resolved via `use_context` up the reactive-owner
    // chain); without this, the pending fetch would re-trigger the page's
    // full-page spinner and the popup would appear only once it resolved.
    // Providing a throwaway local context here shadows the page's for every
    // resource read in this component (render AND the Save-disabling memos), so
    // the popup opens immediately and we drive the loading state ourselves — a
    // disabled select plus a disabled Save button. Nothing renders off this
    // context, so the absorbed registrations are inert.
    #[allow(
        clippy::default_trait_access,
        reason = "SuspenseContext.tasks is leptos's internal slotmap::SlotMap; naming it would add a `slotmap` dependency for a nitpick"
    )]
    provide_context(SuspenseContext {
        tasks: ArcRwSignal::new(Default::default()),
    });

    let title = title(published, current_host.is_some());
    let origin = RwSignal::new(current_host.unwrap_or_default());
    let bucket = RwSignal::new(current_bucket.unwrap_or_default());
    let host_error = RwSignal::new(false);
    let bucket_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);
    // Index into the current workflow view's option list; the whole client-side
    // workflow state. Reset to the view's `initial` whenever the fetched config
    // changes (see the effect below).
    let selected_workflow = RwSignal::new(0usize);

    // The validated (host, bucket) pair the workflow config is keyed on. `Some`
    // only when the bucket is non-empty and the host, if any, is a valid
    // hostname, so the fetch reflects the currently-typed bucket and re-runs
    // when it changes.
    // A published package's bucket is fixed, so it has no workflow to read.
    let valid_target = Memo::new(move |_| {
        if published {
            return None;
        }
        let host = origin.get().trim().to_string();
        let bucket = bucket.get().trim().to_string();
        (field_errors(&host, &bucket) == (false, false)).then_some((host, bucket))
    });

    // Debounced mirror of `valid_target`: the fetch keys on this so it fires
    // once per *settled* bucket name (~400ms after the last keystroke) instead
    // of on every character, which would flash the "Unavailable" notice and hit
    // S3 for each intermediate bucket name. A cleared target propagates
    // immediately (nothing to fetch); a new target supersedes any pending timer.
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

    // Fetch the bucket's workflows for the (debounced) target. `None` (the
    // resource is pending) renders the loading hint; a fetch error degrades to
    // the `Unavailable` notice, matching the commit dialog.
    let workflows = LocalResource::new(move || {
        let target = debounced_target.get();
        async move {
            let (host, bucket) = target?;
            let res = commands::get_bucket_workflows(host.clone(), bucket.clone()).await;
            Some(((host, bucket), res))
        }
    });

    // The workflow view for the current target: this is a first push, so there
    // is no previous revision — pass `None` for the previous workflow id and the
    // bucket default is preselected. `None` here means "no target / still
    // loading"; a fetch error maps to the `Unavailable` view.
    let wf_view = Memo::new(move |_| {
        workflows.get().flatten().and_then(|(target, res)| {
            // Self-keying: ignore a result whose target no longer matches the
            // typed one. Covers BOTH the debounce-pending window and the
            // in-flight-refetch window (the resource keeps the previous bucket's
            // value while re-running), so the view is `None` (loading) until the
            // fetch for the *current* bucket resolves.
            (Some(target) == valid_target.get()).then(|| match res {
                Ok(cw) => build_workflow_view(&cw, None, None),
                Err(_) => build_workflow_view(&CommitWorkflows::Unavailable, None, None),
            })
        })
    });

    // Keep the selection in step with the fetched config: whenever the view
    // changes, restart at its preselected (bucket-default) index so display and
    // submit agree.
    Effect::new(move |_| {
        if let Some(view) = wf_view.get() {
            selected_workflow.set(view.initial);
        }
    });

    // No previous revision on a first push, so the divergence note never shows.
    let workflow_note = Memo::new(|_| None::<String>);

    // True while a valid target's workflow config isn't ready. Because `wf_view`
    // is self-keyed to the current `valid_target` (above), a `None` here covers
    // every not-yet-current window — debounce pending AND in-flight refetch —
    // so blocking Save on it keeps the submitted intent matched to the
    // currently-typed bucket, never a stale one.
    let workflow_loading =
        Memo::new(move |_| valid_target.get().is_some() && wf_view.get().is_none());
    // Disable Save while submitting or while the workflow config is loading, so
    // the submitted intent always corresponds to the displayed bucket's config
    // and never a stale one from an earlier keystroke.
    let save_disabled = Memo::new(move |_| submitting.get() || workflow_loading.get());

    let ns = namespace.clone();
    let on_close_submit = on_close.clone();
    let on_submit = move || {
        // Ignore a submit fired while already submitting, or while the workflow
        // config is still loading — otherwise a fast Save mid-refetch could send
        // a stale intent from a previous bucket.
        if submitting.get_untracked() || workflow_loading.get_untracked() {
            return;
        }
        let origin_val = origin.get_untracked().trim().to_string();
        let bucket_val = bucket.get_untracked().trim().to_string();

        let (host_invalid, bucket_invalid) = field_errors(&origin_val, &bucket_val);
        host_error.set(host_invalid);
        bucket_error.set(bucket_invalid);
        if host_invalid || bucket_invalid {
            return;
        }

        // The selected option's intent, passed straight through — never
        // `Named("")`. Falls back to `BucketDefault` if the config is still
        // loading or unavailable, so the backend resolves the real default.
        let workflow = wf_view
            .get_untracked()
            .and_then(|view| {
                view.options
                    .get(selected_workflow.get_untracked())
                    .map(|o| o.intent.clone())
            })
            .unwrap_or(WorkflowIntent::BucketDefault);

        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        if published {
            leptos::task::spawn_local(async move {
                match commands::change_catalog(ns, origin_val).await {
                    Ok(message) => {
                        notification.set(Some(Notification::Success(message)));
                        on_close();
                        refetch.notify();
                    }
                    Err(e) => {
                        notification.set(Some(Notification::Error(e)));
                        submitting.set(false);
                    }
                }
            });
            return;
        }
        leptos::task::spawn_local(async move {
            match commands::set_remote(ns, origin_val, bucket_val, workflow).await {
                Ok(resp) => {
                    // The remote was set. If its default workflow could not be
                    // resolved, raise a warning carrying the reason instead of a
                    // plain success, so the user knows the package is pushing
                    // ungoverned rather than discovering it only at push time.
                    let notice = match resp.resolution_warning {
                        Some(reason) => Notification::Warning(format!(
                            "Remote set, but the bucket's default workflow couldn't be resolved ({reason}); it will push without a workflow."
                        )),
                        None => Notification::Success(resp.message),
                    };
                    notification.set(Some(notice));
                    on_close();
                    refetch.notify();
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    // Enter on host → focus bucket, or submit once published, when the bucket
    // is locked; Enter on bucket → submit
    let on_submit_host = on_submit.clone();
    let on_submit_bucket = on_submit.clone();
    let on_close_key_host = on_close.clone();
    let on_host_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            if published {
                on_submit_host();
            } else if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = doc.get_element_by_id("set-remote-bucket")
                && let Ok(input) = el.dyn_into::<web_sys::HtmlElement>()
            {
                let _ = input.focus();
            }
        } else if ev.key() == "Escape" {
            on_close_key_host();
        }
    };

    let on_close_key_bucket = on_close.clone();
    let on_bucket_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_bucket();
        } else if ev.key() == "Escape" {
            on_close_key_bucket();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="set-remote-form">
                    <h2 class="section-title">{title}</h2>
                    <div class="set-remote-fields">
                        <div class="set-remote-field">
                            <label>"Host (optional)"</label>
                            <div class="set-remote-input-group">
                                <input
                                    class="set-remote-input"
                                    class:error=move || host_error.get()
                                    type="text"
                                    placeholder="open.quiltdata.com"
                                    prop:value=move || origin.get()
                                    on:input=move |ev| {
                                        origin.set(event_target_value(&ev));
                                        host_error.set(false);
                                    }
                                    on:keydown=on_host_keydown
                                />
                                <span
                                    class="set-remote-hint"
                                    class:visible=move || host_error.get()
                                >
                                    "Enter a valid hostname"
                                </span>
                            </div>
                        </div>

                        <div class="set-remote-field">
                            <label>"Bucket"</label>
                            <div class="set-remote-input-group">
                                <input
                                    id="set-remote-bucket"
                                    class="set-remote-input"
                                    class:error=move || bucket_error.get()
                                    type="text"
                                    placeholder="my-s3-bucket"
                                    prop:disabled=published
                                    prop:value=move || bucket.get()
                                    on:input=move |ev| {
                                        bucket.set(event_target_value(&ev));
                                        bucket_error.set(false);
                                    }
                                    on:keydown=on_bucket_keydown
                                />
                                <span
                                    class="set-remote-hint"
                                    class:visible=move || bucket_error.get()
                                >
                                    "Enter an S3 bucket name"
                                </span>
                            </div>
                        </div>
                    </div>

                    // ── Workflow selector ──
                    // Shown only before the first push: the bucket's
                    // workflow config, fetched for the currently-typed bucket.
                    // A first push has no previous revision, so the bucket
                    // default is preselected.
                    {(!published).then(|| view! {
                        {move || {
                            if valid_target.get().is_none() {
                                // No usable target yet: nothing to fetch or show.
                                ().into_any()
                            } else if let Some(view) = wf_view.get() {
                                view! {
                                    <WorkflowSection
                                        view=view
                                        selected=selected_workflow
                                        note=workflow_note
                                    />
                                }
                                .into_any()
                            } else {
                                // Target set but the config is still loading:
                                // render the control's box with a disabled
                                // select (never a page-level spinner — see the
                                // local SuspenseContext at the top of this
                                // component), so the layout is stable and Save
                                // stays blocked until it loads.
                                view! {
                                    <div class="qui-workflow">
                                        <p class="qui-workflow-field">
                                            <label class="qui-workflow-label" for="workflow">"Workflow"</label>
                                            <select class="qui-workflow-select" id="workflow" name="workflow" disabled>
                                                <option selected>"Loading workflows…"</option>
                                            </select>
                                        </p>
                                    </div>
                                }
                                .into_any()
                            }
                        }}
                    })}

                    // Only an unpublished package that already has a local commit
                    // re-commits on save (creating a new revision); a catalog
                    // change and a commit-less package create nothing.
                    {published.then(|| view! {
                        <p class="set-remote-notice">
                            "A published package keeps its bucket; only its catalog can change."
                        </p>
                    })}
                    {(!published && has_local_commit).then(|| view! {
                        <p class="set-remote-notice">
                            "Setting a remote will create a new revision of the package."
                        </p>
                    })}

                    <div class="set-remote-actions">
                        <buttons::FormPrimary on_click=on_submit_click disabled=save_disabled>
                            "Save"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel>"Cancel"</buttons::FormSecondary>
                    </div>
                </div>
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

    /// A published package keeps its bucket, so the popup is about its
    /// catalog.
    #[test]
    fn the_title_names_what_saving_changes() {
        assert_eq!(title(false, false), "Set remote");
        assert_eq!(title(false, true), "Set remote");
        assert_eq!(title(true, false), "Add catalog");
        assert_eq!(title(true, true), "Change catalog");
    }

    fn mount_popup(current_host: Option<&str>, published: bool) -> web_sys::Element {
        let current_host = current_host.map(str::to_string);
        mount(move || {
            view! {
                <SetRemotePopup
                    namespace="team/survey".to_string()
                    current_host=current_host
                    current_bucket=Some("data-bucket".to_string())
                    published=published
                    has_local_commit=true
                    notification=RwSignal::new(None)
                    refetch=Trigger::new()
                    on_close=|| ()
                />
            }
        })
    }

    /// The host and bucket inputs, in that order.
    fn inputs(el: &web_sys::Element) -> (web_sys::HtmlInputElement, web_sys::HtmlInputElement) {
        let all = el.query_selector_all("input.set-remote-input").unwrap();
        assert_eq!(all.length(), 2, "host and bucket");
        let input = |i| {
            all.item(i)
                .unwrap()
                .unchecked_into::<web_sys::HtmlInputElement>()
        };
        (input(0), input(1))
    }

    /// Once published, only the bucket is locked: the catalog can be added,
    /// and Save stays.
    #[wasm_bindgen_test]
    async fn a_published_package_locks_only_the_bucket() {
        let el = mount_popup(None, true);
        leptos::task::tick().await;
        let (host, bucket) = inputs(&el);

        assert!(!host.disabled(), "the catalog can change");
        assert!(bucket.disabled(), "the bucket is fixed");
        assert_eq!(bucket.value(), "data-bucket");
        element_saying(&el, "Add catalog");
        crate::test_support::button_saying(&el, "Save");
        assert!(
            el.query_selector("select").unwrap().is_none(),
            "no workflow"
        );
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("create a new revision"),
            "a catalog change makes no revision"
        );
    }

    /// Once published, Enter in the catalog field saves: the bucket it would
    /// otherwise move to is locked. With no Tauri host here, the save's
    /// refusal is what shows that it ran.
    #[wasm_bindgen_test]
    async fn enter_in_the_catalog_saves_once_published() {
        let notification = RwSignal::new(None);
        let el = mount(move || {
            view! {
                <SetRemotePopup
                    namespace="team/survey".to_string()
                    current_host=None
                    current_bucket=Some("data-bucket".to_string())
                    published=true
                    notification=notification
                    refetch=Trigger::new()
                    on_close=|| ()
                />
            }
        });
        leptos::task::tick().await;
        let (host, _) = inputs(&el);
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Enter");
        host.dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
        crate::test_support::sleep_ms(50).await;

        assert!(
            matches!(notification.get_untracked(), Some(Notification::Error(_))),
            "the save ran"
        );
    }

    /// Before the first push both fields are open, as a first remote is.
    #[wasm_bindgen_test]
    fn an_unpublished_package_opens_both_fields() {
        let el = mount_popup(Some("quilt.test"), false);
        let (host, bucket) = inputs(&el);

        assert!(!host.disabled());
        assert!(!bucket.disabled());
        element_saying(&el, "Set remote");
    }

    /// A host, when given, must be a hostname; a bucket must be something.
    #[test]
    fn the_form_flags_each_wrong_field() {
        assert_eq!(field_errors("", ""), (false, true));
        assert_eq!(field_errors("not a host", "b"), (true, false));
        assert_eq!(field_errors("quilt.test", "my-bucket"), (false, false));
    }

    /// The host is optional: a bucket with no catalog is reached on this
    /// computer's AWS credentials.
    #[test]
    fn an_empty_host_is_accepted() {
        assert_eq!(field_errors("", "my-bucket"), (false, false));
    }
}

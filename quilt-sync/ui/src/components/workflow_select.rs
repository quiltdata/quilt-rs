//! Shared workflow-selection control.
//!
//! The backend's [`CommitWorkflows`] state maps to a render model whose option
//! list carries, per entry, the [`WorkflowIntent`] it submits. The selected
//! index into that list is the whole client-side state. Both the commit dialog
//! (`pages::commit`) and the set-remote popup (`components::set_remote_popup`)
//! reuse this so their dropdowns are byte-for-byte the same catalog-parity
//! control.

use leptos::prelude::*;

use crate::commands::{CommitWorkflows, WorkflowData, WorkflowInfo, WorkflowIntent};

// ── Workflow dropdown model ──

/// One entry in the workflow dropdown. Carries its own display label and the
/// [`WorkflowIntent`] it submits, so the handler passes the selected option's
/// intent straight through — no re-derivation, never `Named("")`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowOption {
    pub label: String,
    pub intent: WorkflowIntent,
    pub disabled: bool,
}

/// Display label for a single declared workflow: its name, or its id when the
/// config gave no name.
fn workflow_label(w: &WorkflowInfo) -> String {
    w.name.clone().unwrap_or_else(|| w.id.clone())
}

/// The previous revision's workflow stamp, distinguishing the three cases the
/// override note must tell apart.
///
/// `preselected_index` only needs the concrete `Named` id, but the note also
/// cares whether the previous revision was never pushed versus explicitly
/// stamped with no workflow — so this is the one place that distinction lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviousWorkflow {
    NeverPushed,
    ExplicitNone,
    Named(String),
}

impl PreviousWorkflow {
    /// Derive from the previous revision's stamp: absent → never pushed, present
    /// with no id → an explicit no-workflow choice, present with an id → that
    /// workflow.
    pub fn from_stamp(workflow: Option<&WorkflowData>) -> Self {
        match workflow {
            None => Self::NeverPushed,
            Some(WorkflowData { id: None }) => Self::ExplicitNone,
            Some(WorkflowData { id: Some(id) }) => Self::Named(id.clone()),
        }
    }

    /// The workflow id `preselected_index` treats as the previous pick: only a
    /// concrete named workflow, collapsing never-pushed and explicit-none (the
    /// same behavior preselection had before this note existed).
    pub fn preselect_id(&self) -> Option<&str> {
        match self {
            Self::Named(id) => Some(id),
            Self::NeverPushed | Self::ExplicitNone => None,
        }
    }
}

/// Neutral note shown under the dropdown when the currently-selected workflow
/// diverges from what the previous revision stamped, so the override (the
/// bucket default winning over the previous pick in `preselected_index`) is
/// visible. `None` means no divergence to report.
///
/// Pure and signal-free: the render layer recomputes it whenever the selection
/// changes.
pub fn previous_workflow_note(
    previous: &PreviousWorkflow,
    current: &WorkflowIntent,
    workflows: &[WorkflowInfo],
) -> Option<String> {
    match previous {
        // Nothing was ever stamped, so nothing can be overridden.
        PreviousWorkflow::NeverPushed => None,
        PreviousWorkflow::ExplicitNone => match current {
            WorkflowIntent::NoWorkflow => None,
            WorkflowIntent::BucketDefault | WorkflowIntent::Named(_) => {
                Some("The previous revision used no workflow.".to_string())
            }
        },
        PreviousWorkflow::Named(prev) => {
            if matches!(current, WorkflowIntent::Named(id) if id == prev) {
                None
            } else {
                let label = workflows
                    .iter()
                    .find(|w| &w.id == prev)
                    .map_or_else(|| prev.clone(), workflow_label);
                Some(format!(
                    "The previous revision used the \"{label}\" workflow."
                ))
            }
        }
    }
}

/// Build the dropdown's option list for the [`CommitWorkflows::Available`]
/// state, matching the web catalog's order:
/// - index 0: `None` → [`WorkflowIntent::NoWorkflow`], disabled when a workflow
///   is required. Its label gains `" (default)"` when the config names no
///   default and no workflow is required — then `None` is itself the bucket
///   default (the resolver treats `BucketDefault` and `NoWorkflow` alike);
/// - one entry per declared workflow (label = name or id, with `" (default)"`
///   appended to the bucket default) → [`WorkflowIntent::Named`].
///
/// Exactly one option ever bears `" (default)"`: whichever is the bucket's
/// actual default. A required-but-defaultless bucket has none.
///
/// There is deliberately no `Bucket default` item — the catalog offers only
/// `None` plus the concrete workflows. An Available-but-empty workflow list
/// therefore yields just `[None]`, which is fine.
pub fn workflow_options(
    workflows: &[WorkflowInfo],
    default_workflow: Option<&str>,
    is_workflow_required: bool,
) -> Vec<WorkflowOption> {
    // When the config names no default and doesn't require a workflow, `None`
    // IS the bucket default (the resolver treats `BucketDefault` and
    // `NoWorkflow` identically then), so it bears the `(default)` marker.
    // A required-but-defaultless bucket leaves `None` disabled with the
    // required error, so it must not be labeled default.
    let none_is_default = default_workflow.is_none() && !is_workflow_required;
    let none_label = if none_is_default {
        "None (default)".to_string()
    } else {
        "None".to_string()
    };
    let mut options = vec![WorkflowOption {
        label: none_label,
        intent: WorkflowIntent::NoWorkflow,
        disabled: is_workflow_required,
    }];

    options.extend(workflows.iter().map(|w| {
        let is_default = default_workflow.is_some_and(|id| id == w.id);
        let label = if is_default {
            format!("{} (default)", workflow_label(w))
        } else {
            workflow_label(w)
        };
        WorkflowOption {
            label,
            intent: WorkflowIntent::Named(w.id.clone()),
            disabled: false,
        }
    }));

    options
}

/// Index (into [`workflow_options`]'s output) that should start selected in the
/// [`CommitWorkflows::Available`] state.
///
/// The bucket default always wins over the previous revision's pick — that is
/// what makes it the default. This intentionally diverges from the web catalog,
/// which preselects the previous revision's workflow first. That rule dates to
/// the 2020 package-update dialog (quilt#1856), which chose to reuse the
/// previous revision's workflow even though `default_workflow` already existed —
/// a deliberate but lightly-justified UX choice (a one-line "reuse previous"
/// comment, no recorded rationale). We prefer the bucket default here instead —
/// it also heals packages whose workflow was set through the earlier error-prone
/// free-text field: the next commit adopts what the bucket intends rather than
/// carrying a likely-wrong prior stamp forward. Options are laid out as
/// `[None, workflows..]`, so
/// a workflow at position `p` maps to option index `p + 1`. The rule mirrors the
/// `(default)` marker that [`workflow_options`] applies:
///
/// 1. A named `default_workflow` present in the list → its index (ignoring the
///    previous pick).
/// 2. No named default and the bucket doesn't require a workflow → index 0, the
///    `None` head, which is itself labeled `None (default)` and so is the
///    effective default.
/// 3. Required bucket with no usable default (genuinely no default): the
///    previous revision's workflow if present, else index 0 (`None`, disabled).
///
/// A first push (set-remote popup) has no previous revision, so it passes
/// `previous_workflow_id = None` and the bucket default is preselected.
pub fn preselected_index(
    workflows: &[WorkflowInfo],
    previous_workflow_id: Option<&str>,
    default_workflow: Option<&str>,
    is_workflow_required: bool,
) -> usize {
    // 1. A named default in the list wins outright.
    if let Some(def) = default_workflow
        && let Some(pos) = workflows.iter().position(|w| w.id == def)
    {
        return pos + 1;
    }
    // 2. No named default and no requirement: `None` is itself the default
    //    (labeled "None (default)"), so it wins too — keeping "the default
    //    always wins" uniform.
    if !is_workflow_required {
        return 0;
    }
    // 3. Required with genuinely no default: fall back to the previous pick.
    if let Some(prev) = previous_workflow_id
        && let Some(pos) = workflows.iter().position(|w| w.id == prev)
    {
        return pos + 1;
    }
    // The `None` item sits at the head of the option list.
    0
}

/// Which of the three [`CommitWorkflows`] states the section renders, plus the
/// per-state chrome the render needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowViewKind {
    /// The bucket has a config: render the dropdown. Carries whether the bucket
    /// requires a workflow (drives the "required" hint).
    Available { is_workflow_required: bool },
    /// The bucket is ungoverned: a single disabled `None`, no choice to make.
    NotConfigured,
    /// The config couldn't be loaded (transient/network): an inline soft notice,
    /// no dropdown. Commit re-resolves the bucket default.
    Unavailable,
    /// The config is malformed: an inline notice naming the reason. Commits to
    /// this bucket will fail until it is fixed, so the notice must not promise a
    /// fallback.
    Invalid { reason: String },
}

/// The workflow section, resolved from the backend state.
///
/// `options` and `initial` drive both display and submit uniformly across all
/// three states: submit reads `options[selected].intent`. On the non-`Available`
/// states this is a single [`WorkflowIntent::BucketDefault`] entry at index 0,
/// so a config-less bucket resolves to no-workflow while a transient failure
/// re-resolves the real default at commit time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowView {
    pub kind: WorkflowViewKind,
    pub options: Vec<WorkflowOption>,
    pub initial: usize,
}

/// Map the backend [`CommitWorkflows`] state to the section's render model.
pub fn build_workflow_view(
    workflows: &CommitWorkflows,
    previous_workflow_id: Option<&str>,
) -> WorkflowView {
    match workflows {
        CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
        } => WorkflowView {
            kind: WorkflowViewKind::Available {
                is_workflow_required: *is_workflow_required,
            },
            options: workflow_options(
                workflows,
                default_workflow.as_deref(),
                *is_workflow_required,
            ),
            initial: preselected_index(
                workflows,
                previous_workflow_id,
                default_workflow.as_deref(),
                *is_workflow_required,
            ),
        },
        // Ungoverned bucket: a single disabled `None`. Submit sends
        // `BucketDefault`, which on a config-less bucket resolves to
        // no-workflow — the honest "let the bucket decide".
        CommitWorkflows::NotConfigured => WorkflowView {
            kind: WorkflowViewKind::NotConfigured,
            options: vec![WorkflowOption {
                label: "None".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }],
            initial: 0,
        },
        // Config load failed (transient): no dropdown, just a notice. Submit
        // sends `BucketDefault` so the real default re-resolves at commit time
        // (the network may have recovered) rather than forcing no-workflow.
        CommitWorkflows::Unavailable => WorkflowView {
            kind: WorkflowViewKind::Unavailable,
            options: vec![WorkflowOption {
                label: "Bucket default".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }],
            initial: 0,
        },
        // Config is malformed: no dropdown, a notice naming the reason. Submit
        // still carries `BucketDefault`; the commit will fail loudly against the
        // invalid config (semantics unchanged), but the user is told up front.
        CommitWorkflows::Invalid { reason } => WorkflowView {
            kind: WorkflowViewKind::Invalid {
                reason: reason.clone(),
            },
            options: vec![WorkflowOption {
                label: "Bucket default".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }],
            initial: 0,
        },
    }
}

// ── Workflow section ──

#[component]
pub fn WorkflowSection(
    view: WorkflowView,
    selected: RwSignal<usize>,
    /// Neutral divergence note, live over `selected`. Rendered only in the
    /// `Available` state; the other states have no dropdown to override. A first
    /// push has no previous revision, so the popup passes a note that is always
    /// `None`.
    note: Memo<Option<String>>,
) -> impl IntoView {
    let WorkflowView {
        kind,
        options,
        initial,
    } = view;

    match kind {
        WorkflowViewKind::Available {
            is_workflow_required,
        } => workflow_dropdown(options, selected, initial, is_workflow_required, note).into_any(),
        // Ungoverned bucket: a single disabled `None`, plus a hint explaining
        // why there is no choice to make. Submit already carries `BucketDefault`
        // via `options[0]`.
        WorkflowViewKind::NotConfigured => view! {
            <div class="qui-workflow">
                <p class="qui-workflow-field">
                    <label class="qui-workflow-label" for="workflow">"Workflow"</label>
                    <select class="qui-workflow-select" id="workflow" name="workflow" disabled>
                        <option selected>"None"</option>
                    </select>
                </p>
                <span class="qui-workflow-hint">"This bucket has no workflow configuration."</span>
            </div>
        }
        .into_any(),
        // Config load failed (transient): no dropdown, just an inline soft
        // notice. Submit does not block; it re-resolves the bucket default at
        // commit time. The notice does NOT promise the default will apply —
        // only that the commit will try it — since the load may keep failing.
        WorkflowViewKind::Unavailable => view! {
            <div class="qui-workflow">
                <p class="qui-workflow-field">
                    <label class="qui-workflow-label" for="workflow">"Workflow"</label>
                    <span class="qui-workflow-hint">
                        "⚠ Couldn't load this bucket's workflows. Commit will try the bucket's default workflow."
                    </span>
                </p>
            </div>
        }
        .into_any(),
        // Config is malformed: an inline notice that names the reason and warns
        // that commits will fail until the config is fixed — no false promise
        // of a bucket-default fallback.
        WorkflowViewKind::Invalid { reason } => view! {
            <div class="qui-workflow">
                <p class="qui-workflow-field">
                    <label class="qui-workflow-label" for="workflow">"Workflow"</label>
                    <span class="qui-workflow-error">
                        {format!("⚠ This bucket's workflow configuration is invalid ({reason}). Commits to this bucket will fail until it's fixed.")}
                    </span>
                </p>
            </div>
        }
        .into_any(),
    }
}

/// The [`CommitWorkflows::Available`] dropdown.
fn workflow_dropdown(
    options: Vec<WorkflowOption>,
    selected: RwSignal<usize>,
    initial: usize,
    is_workflow_required: bool,
    note: Memo<Option<String>>,
) -> impl IntoView {
    // Intents indexed by option position — used to decide whether the current
    // selection is the (disabled) `None` item, which drives the required hint.
    let intents: Vec<WorkflowIntent> = options.iter().map(|o| o.intent.clone()).collect();
    let show_required_hint = move || {
        is_workflow_required
            && intents
                .get(selected.get())
                .is_some_and(|i| *i == WorkflowIntent::NoWorkflow)
    };

    // The initial selection is rendered as the HTML boolean `selected`
    // attribute on the matching option. Unlike a `<select prop:value=…>`
    // (which tachys applies before the options mount, making it a no-op), the
    // attribute survives mount order and works even on the disabled `None`
    // option. `initial` is the same index that seeds `selected` in the parent,
    // so what the browser shows and what submit sends cannot diverge. The
    // `on:change` handler keeps the `selected` signal in step thereafter.
    let option_views = options
        .into_iter()
        .enumerate()
        .map(|(i, o)| {
            view! {
                <option value=i.to_string() selected=i == initial disabled=o.disabled>
                    {o.label}
                </option>
            }
        })
        .collect::<Vec<_>>();

    view! {
        <div class="qui-workflow">
            <p class="qui-workflow-field">
                <label class="qui-workflow-label" for="workflow">"Workflow"</label>
                <select
                    class="qui-workflow-select"
                    id="workflow"
                    name="workflow"
                    on:change=move |ev| {
                        if let Ok(i) = event_target_value(&ev).parse::<usize>() {
                            selected.set(i);
                        }
                    }
                >
                    {option_views}
                </select>
            </p>
            <Show when=show_required_hint>
                <span class="qui-workflow-error">"Workflow is required for this bucket."</span>
            </Show>
            // Neutral note: appears when the current pick diverges from the
            // previous revision's stamp, and disappears when they match again.
            {move || {
                note.get().map(|text| view! { <p class="qui-workflow-hint">{text}</p> })
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PreviousWorkflow, WorkflowOption, WorkflowViewKind, build_workflow_view, preselected_index,
        previous_workflow_note, workflow_options,
    };
    use crate::commands::{CommitWorkflows, WorkflowData, WorkflowInfo, WorkflowIntent};

    fn wf(id: &str, name: Option<&str>) -> WorkflowInfo {
        WorkflowInfo {
            id: id.to_string(),
            name: name.map(str::to_string),
            description: None,
        }
    }

    // ── Normal path (workflows non-empty) ──

    #[test]
    fn options_normal_none_head_default_labeled_no_bucket_default() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        assert_eq!(
            opts,
            vec![
                WorkflowOption {
                    label: "None".to_string(),
                    intent: WorkflowIntent::NoWorkflow,
                    disabled: false,
                },
                WorkflowOption {
                    // The bucket default gets the " (default)" suffix.
                    label: "Alpha WF (default)".to_string(),
                    intent: WorkflowIntent::Named("alpha".to_string()),
                    disabled: false,
                },
                WorkflowOption {
                    // No name → fall back to the id as the label; not the default.
                    label: "beta".to_string(),
                    intent: WorkflowIntent::Named("beta".to_string()),
                    disabled: false,
                },
            ]
        );
        // The normal path never offers a `Bucket default` item.
        assert!(
            opts.iter()
                .all(|o| o.intent != WorkflowIntent::BucketDefault)
        );
    }

    #[test]
    fn options_default_suffix_uses_label_or_id() {
        // Unnamed default → suffix appended to the id.
        let wfs = vec![wf("beta", None)];
        assert_eq!(
            workflow_options(&wfs, Some("beta"), false)[1].label,
            "beta (default)"
        );
        // Default id not present in the list → no option carries the suffix.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        let opts = workflow_options(&wfs, Some("ghost"), false);
        assert_eq!(opts[1].label, "Alpha WF");
        assert!(opts.iter().all(|o| !o.label.contains("(default)")));
    }

    #[test]
    fn options_no_default_not_required_labels_none_as_default() {
        // No named default and not required → `None` IS the bucket default, so
        // it carries the suffix; the concrete workflow does not.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        let opts = workflow_options(&wfs, None, false);
        assert_eq!(opts[0].label, "None (default)");
        assert_eq!(opts[1].label, "Alpha WF");
    }

    /// Number of labels ending in the `(default)` marker across the option list.
    fn default_marker_count(opts: &[WorkflowOption]) -> usize {
        opts.iter()
            .filter(|o| o.label.ends_with("(default)"))
            .count()
    }

    #[test]
    fn options_exactly_one_default_marker_when_named_default_present() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        // The named default is the real workflow; `None` stays plain.
        assert_eq!(opts[0].label, "None");
        assert_eq!(default_marker_count(&opts), 1);
        assert_eq!(opts[1].label, "Alpha WF (default)");
    }

    #[test]
    fn options_exactly_one_default_marker_when_no_default_not_required() {
        // No named default and not required → `None` is the only default.
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, None, false);
        assert_eq!(default_marker_count(&opts), 1);
        assert_eq!(opts[0].label, "None (default)");
    }

    #[test]
    fn options_no_default_marker_when_no_default_but_required() {
        // Required with no named default → `None` is disabled with the required
        // error, so it must NOT be labeled default; there genuinely is none.
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, None, true);
        assert_eq!(default_marker_count(&opts), 0);
        assert_eq!(opts[0].label, "None");
        assert!(opts[0].disabled);
    }

    #[test]
    fn options_no_default_marker_when_named_default_absent_from_list() {
        // Stale/misconfigured default id not in the list → the real-workflow
        // branch adds no marker, and `None` gets none either (default is Some).
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("ghost"), false);
        assert_eq!(default_marker_count(&opts), 0);
        assert_eq!(opts[0].label, "None");
    }

    #[test]
    fn options_none_disabled_iff_required() {
        let wfs = vec![wf("alpha", None)];
        let required = workflow_options(&wfs, None, true);
        assert_eq!(required[0].intent, WorkflowIntent::NoWorkflow);
        assert!(required[0].disabled);
        assert!(!workflow_options(&wfs, None, false)[0].disabled);
    }

    #[test]
    fn options_available_empty_list_is_just_none() {
        // An Available-but-empty workflow list yields just `[None]` (labeled the
        // default, since nothing is required and no default is named).
        let opts = workflow_options(&[], None, false);
        assert_eq!(
            opts,
            vec![WorkflowOption {
                label: "None (default)".to_string(),
                intent: WorkflowIntent::NoWorkflow,
                disabled: false,
            }]
        );
    }

    // ── State → render model (`build_workflow_view`) ──

    #[test]
    fn view_available_builds_dropdown_options() {
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: false,
        };
        let view = build_workflow_view(&workflows, Some("beta"));
        assert_eq!(
            view.kind,
            WorkflowViewKind::Available {
                is_workflow_required: false
            }
        );
        // Options + preselection match the pure builders for the Available case.
        // Previous picked "beta", but the named default "alpha" wins.
        assert_eq!(view.options.len(), 3);
        assert_eq!(view.initial, 1);
        assert_eq!(
            view.options[view.initial].intent,
            WorkflowIntent::Named("alpha".to_string())
        );
    }

    #[test]
    fn view_not_configured_is_single_disabled_none_submitting_bucket_default() {
        let view = build_workflow_view(&CommitWorkflows::NotConfigured, None);
        assert_eq!(view.kind, WorkflowViewKind::NotConfigured);
        assert_eq!(view.initial, 0);
        assert_eq!(
            view.options,
            vec![WorkflowOption {
                label: "None".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }]
        );
    }

    #[test]
    fn view_unavailable_submits_bucket_default() {
        let view = build_workflow_view(&CommitWorkflows::Unavailable, Some("ignored"));
        assert_eq!(view.kind, WorkflowViewKind::Unavailable);
        assert_eq!(view.initial, 0);
        // Both non-Available states submit `BucketDefault`, but they are
        // distinct render inputs (NotConfigured vs Unavailable).
        assert_eq!(view.options[0].intent, WorkflowIntent::BucketDefault);
    }

    #[test]
    fn view_invalid_carries_reason_and_submits_bucket_default() {
        // A malformed config maps to a distinct render kind carrying the
        // reason, so the notice can name it; submit still sends `BucketDefault`.
        let view = build_workflow_view(
            &CommitWorkflows::Invalid {
                reason: "bad schema".to_string(),
            },
            Some("ignored"),
        );
        assert_eq!(
            view.kind,
            WorkflowViewKind::Invalid {
                reason: "bad schema".to_string(),
            }
        );
        assert_eq!(view.initial, 0);
        assert_eq!(view.options[0].intent, WorkflowIntent::BucketDefault);
    }

    // ── Preselection: the bucket default always wins over the previous pick ──

    #[test]
    fn preselect_named_default_wins_over_previous() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Options: [None=0, alpha=1, beta=2]. Previous picked beta, but the
        // named default alpha wins — that's why it is the default.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("alpha"), false),
            1
        );
        // Requiredness doesn't change it: a named default still wins.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("alpha"), true),
            1
        );
    }

    #[test]
    fn preselect_none_default_wins_when_not_required() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // No named default, not required → `None` is the effective default
        // ("None (default)") and wins over the previous pick.
        assert_eq!(preselected_index(&wfs, Some("beta"), None, false), 0);
        // A named default absent from the list is no usable default; not
        // required → `None` still wins.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("ghost"), false),
            0
        );
    }

    #[test]
    fn preselect_required_no_default_falls_back_to_previous() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Required with genuinely no default: the previous revision's workflow.
        assert_eq!(preselected_index(&wfs, Some("beta"), None, true), 2);
        // A named default absent from the list is still no default.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("ghost"), true),
            2
        );
        // Required, no default, no previous → the `None` head (disabled).
        assert_eq!(preselected_index(&wfs, None, None, true), 0);
        // Required, no default, previous absent from the list → `None` head.
        assert_eq!(preselected_index(&wfs, Some("ghost"), None, true), 0);
    }

    // ── First push (set-remote popup): no previous → bucket default ──

    #[test]
    fn preselect_first_push_lands_on_bucket_default() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];

        // Named default, no previous revision → the named default is preselected.
        assert_eq!(preselected_index(&wfs, None, Some("beta"), false), 2);
        assert_eq!(preselected_index(&wfs, None, Some("beta"), true), 2);

        // No named default and not required → the `None (default)` head.
        assert_eq!(preselected_index(&wfs, None, None, false), 0);

        // Required with no default and no previous → the disabled `None` head;
        // the user must actively pick a workflow.
        assert_eq!(preselected_index(&wfs, None, None, true), 0);
    }

    #[test]
    fn view_first_push_available_preselects_bucket_default() {
        // A first push feeds `None` for the previous workflow id; the resulting
        // selection is the bucket default, never `Named("")`.
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: Some("beta".to_string()),
            is_workflow_required: true,
        };
        let view = build_workflow_view(&workflows, None);
        assert_eq!(view.initial, 2);
        assert_eq!(
            view.options[view.initial].intent,
            WorkflowIntent::Named("beta".to_string())
        );
    }

    // ── Consistency invariant: preselection lands on the "(default)" option ──

    #[test]
    fn preselect_agrees_with_default_label() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];

        // Case 1: named default present → selected option bears "(default)".
        let opts = workflow_options(&wfs, Some("alpha"), false);
        let idx = preselected_index(&wfs, Some("beta"), Some("alpha"), false);
        assert!(opts[idx].label.ends_with("(default)"));
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));

        // Case 2: no named default, not required → the `None` head is
        // "None (default)" and is the selection.
        let opts = workflow_options(&wfs, None, false);
        let idx = preselected_index(&wfs, Some("beta"), None, false);
        assert_eq!(idx, 0);
        assert_eq!(opts[0].label, "None (default)");

        // Case 3: required, no default → no option bears "(default)"; the
        // selection is the previous pick (or `None`).
        let opts = workflow_options(&wfs, None, true);
        assert!(opts.iter().all(|o| !o.label.ends_with("(default)")));
        let idx = preselected_index(&wfs, Some("beta"), None, true);
        assert_eq!(idx, 2);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("beta".to_string()));
    }

    // ── Display == submit invariant, guarded at the pure-fn level ──

    #[test]
    fn preselected_index_maps_to_submitted_intent_normal() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        // Previous revision picked "beta", but the named default "alpha" wins:
        // the rendered+submitted option is the labeled default.
        let idx = preselected_index(&wfs, Some("beta"), Some("alpha"), false);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));
        assert_eq!(opts[idx].label, "Alpha WF (default)");
        // No previous id: preselection still lands on the labeled default.
        let idx = preselected_index(&wfs, None, Some("alpha"), false);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));
        assert_eq!(opts[idx].label, "Alpha WF (default)");
    }

    // ── Previous-workflow override note (pure) ──

    #[test]
    fn note_never_pushed_is_silent() {
        // No prior stamp → nothing can be overridden, regardless of selection.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        for current in [
            WorkflowIntent::BucketDefault,
            WorkflowIntent::NoWorkflow,
            WorkflowIntent::Named("alpha".to_string()),
        ] {
            assert_eq!(
                previous_workflow_note(&PreviousWorkflow::NeverPushed, &current, &wfs),
                None
            );
        }
    }

    #[test]
    fn note_explicit_none_silent_when_current_is_no_workflow() {
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::ExplicitNone,
                &WorkflowIntent::NoWorkflow,
                &wfs
            ),
            None
        );
    }

    #[test]
    fn note_explicit_none_flags_divergence_to_a_workflow() {
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        // Current selection is a named workflow → the previous no-workflow is
        // being overridden.
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::ExplicitNone,
                &WorkflowIntent::Named("alpha".to_string()),
                &wfs
            ),
            Some("The previous revision used no workflow.".to_string())
        );
        // BucketDefault likewise diverges from an explicit no-workflow.
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::ExplicitNone,
                &WorkflowIntent::BucketDefault,
                &wfs
            ),
            Some("The previous revision used no workflow.".to_string())
        );
    }

    #[test]
    fn note_named_silent_when_current_matches_previous() {
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::Named("alpha".to_string()),
                &WorkflowIntent::Named("alpha".to_string()),
                &wfs
            ),
            None
        );
    }

    #[test]
    fn note_named_uses_workflow_name_when_in_list() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        // Current selection differs (another named workflow) → show the
        // previous workflow's display name.
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::Named("alpha".to_string()),
                &WorkflowIntent::Named("beta".to_string()),
                &wfs
            ),
            Some("The previous revision used the \"Alpha WF\" workflow.".to_string())
        );
        // BucketDefault also diverges from the previous named pick; unnamed
        // workflow falls back to its id as the label.
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::Named("beta".to_string()),
                &WorkflowIntent::BucketDefault,
                &wfs
            ),
            Some("The previous revision used the \"beta\" workflow.".to_string())
        );
    }

    #[test]
    fn note_named_falls_back_to_id_when_absent_from_list() {
        // Previous workflow id no longer declared → use the raw id in the note.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        assert_eq!(
            previous_workflow_note(
                &PreviousWorkflow::Named("ghost".to_string()),
                &WorkflowIntent::Named("alpha".to_string()),
                &wfs
            ),
            Some("The previous revision used the \"ghost\" workflow.".to_string())
        );
    }

    #[test]
    fn previous_workflow_from_stamp_and_preselect_id() {
        // Never pushed / explicit-none collapse to no preselect id; a named
        // stamp yields exactly that id (preselection behavior is unchanged).
        assert_eq!(
            PreviousWorkflow::from_stamp(None),
            PreviousWorkflow::NeverPushed
        );
        assert_eq!(
            PreviousWorkflow::from_stamp(Some(&WorkflowData { id: None })),
            PreviousWorkflow::ExplicitNone
        );
        assert_eq!(
            PreviousWorkflow::from_stamp(Some(&WorkflowData {
                id: Some("alpha".to_string())
            })),
            PreviousWorkflow::Named("alpha".to_string())
        );
        assert_eq!(PreviousWorkflow::NeverPushed.preselect_id(), None);
        assert_eq!(PreviousWorkflow::ExplicitNone.preselect_id(), None);
        assert_eq!(
            PreviousWorkflow::Named("alpha".to_string()).preselect_id(),
            Some("alpha")
        );
    }

    #[test]
    fn view_non_available_states_submit_bucket_default() {
        // The submit path reads `options[selected].intent`; both non-Available
        // states must send `BucketDefault` from their single option at index 0.
        for state in [CommitWorkflows::NotConfigured, CommitWorkflows::Unavailable] {
            let view = build_workflow_view(&state, None);
            assert_eq!(
                view.options[view.initial].intent,
                WorkflowIntent::BucketDefault
            );
        }
    }
}

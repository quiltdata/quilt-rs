//! The workflow choice's model, shared by the commit page and the package
//! page's bucket form.
//!
//! The backend's [`CommitWorkflows`] state maps to a render model whose option
//! list carries, per entry, the [`WorkflowIntent`] it submits. The selected
//! index into that list is the whole client-side state.

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

/// The previous revision's workflow stamp: never pushed, explicitly stamped
/// with no workflow, or a named workflow.
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
    /// concrete named workflow, collapsing never-pushed and explicit-none.
    pub fn preselect_id(&self) -> Option<&str> {
        match self {
            Self::Named(id) => Some(id),
            Self::NeverPushed | Self::ExplicitNone => None,
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
/// A first push (the bucket form) has no previous revision, so it passes
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

/// Which of the [`CommitWorkflows`] states the choice renders, plus what each
/// state's render needs.
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

/// The workflow choice, resolved from the backend state.
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

/// Map the backend [`CommitWorkflows`] state to the choice's render model.
///
/// `settings_workflow_id` is the publish settings' default workflow, which the
/// commit page preselects first so it starts from what one-click Publish
/// would send. It applies only when the bucket declares it; otherwise
/// [`preselected_index`] decides, unchanged. Callers with no publish to mirror
/// pass `None`.
pub fn build_workflow_view(
    workflows: &CommitWorkflows,
    previous_workflow_id: Option<&str>,
    settings_workflow_id: Option<&str>,
) -> WorkflowView {
    match workflows {
        CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
            ..
        } => WorkflowView {
            kind: WorkflowViewKind::Available {
                is_workflow_required: *is_workflow_required,
            },
            options: workflow_options(
                workflows,
                default_workflow.as_deref(),
                *is_workflow_required,
            ),
            initial: settings_workflow_id
                .and_then(|id| workflows.iter().position(|w| w.id == id))
                // Options are laid out as `[None, workflows..]`.
                .map_or_else(
                    || {
                        preselected_index(
                            workflows,
                            previous_workflow_id,
                            default_workflow.as_deref(),
                            *is_workflow_required,
                        )
                    },
                    |pos| pos + 1,
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
        CommitWorkflows::Invalid { reason, .. } => WorkflowView {
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

// ── Settings hint ──

/// Whether the dropdown says its selection comes from Settings: only while
/// the selected workflow is the publish settings' default. With no default
/// workflow in Settings there is nothing to explain, and the form doesn't
/// nudge the user to set one.
pub fn shows_settings_workflow_hint(
    selected: Option<&WorkflowIntent>,
    settings_workflow: Option<&str>,
) -> bool {
    matches!(
        (selected, settings_workflow),
        (Some(WorkflowIntent::Named(id)), Some(settings)) if id == settings
    )
}

/// The publish settings' default workflow when this bucket can't use it:
/// it isn't among the declared workflows, or the bucket has no workflows
/// config. One-click Publish sends it as is and fails here, while the form
/// falls back to the bucket's preselection, so the form says so. `None` when
/// Settings names no workflow, the bucket declares it, or the config couldn't
/// be read (nothing is known either way).
pub fn missing_settings_workflow<'a>(
    view: &WorkflowView,
    settings_workflow: Option<&'a str>,
) -> Option<&'a str> {
    let id = settings_workflow?;
    match view.kind {
        WorkflowViewKind::Available { .. } => (!view
            .options
            .iter()
            .any(|o| matches!(&o.intent, WorkflowIntent::Named(named) if named == id)))
        .then_some(id),
        WorkflowViewKind::NotConfigured => Some(id),
        WorkflowViewKind::Unavailable | WorkflowViewKind::Invalid { .. } => None,
    }
}

// ── The select ──

/// What the primary says while a required workflow is still unchosen.
pub(crate) const CHOOSE_A_WORKFLOW: &str =
    "This bucket requires a workflow. Choose one to publish.";

/// The select's options and where it starts, from the [`WorkflowView`]. The
/// commit page and the bucket form both draw their select from this.
///
/// The select cannot grey an option, so a disabled one (`None` on a bucket
/// that requires a workflow) is left out — unless the view starts on it, which
/// it does on a required bucket that names no default: then it stays, at the
/// head, so the reader still has to choose, and `needs_choice` says so. That
/// holds with one workflow as with several: the form never chooses for them.
///
/// Labels are made unique, since two workflows may share a name: the label
/// is how the select names an option, and the draft keeps the intent.
pub(crate) fn workflow_choices(view: &WorkflowView) -> (Vec<WorkflowOption>, WorkflowIntent, bool) {
    let enabled: Vec<WorkflowOption> = view
        .options
        .iter()
        .filter(|o| !o.disabled)
        .cloned()
        .collect();
    let start = view.options.get(view.initial);
    let (mut options, initial, needs_choice) = match start {
        Some(start) if start.disabled && !enabled.is_empty() => {
            let mut options = vec![start.clone()];
            options.extend(enabled);
            (options, start.intent.clone(), true)
        }
        Some(start) => (enabled, start.intent.clone(), false),
        None => (enabled, WorkflowIntent::BucketDefault, false),
    };
    unique_labels(&mut options);
    (options, initial, needs_choice)
}

/// The labels the select offers. The unchosen start, at the head, goes once
/// the reader has chosen, so it cannot be picked back.
pub(crate) fn offered(options: &[WorkflowOption], needs_choice: bool, chosen: bool) -> Vec<String> {
    options
        .iter()
        .skip(usize::from(needs_choice && chosen))
        .map(|o| o.label.clone())
        .collect()
}

/// Give every option its own label, so the select's label names exactly one
/// intent. A name two workflows share gets the workflow's id; anything that
/// still collides — a workflow whose own name reads like that, say — gets a
/// counter. The first holder of a label keeps it as it is.
fn unique_labels(options: &mut [WorkflowOption]) {
    let shared: Vec<bool> = options
        .iter()
        .map(|o| options.iter().filter(|p| p.label == o.label).count() > 1)
        .collect();
    let mut used = std::collections::HashSet::new();
    for (option, shared) in options.iter_mut().zip(shared) {
        let mut label = match &option.intent {
            WorkflowIntent::Named(id) if shared => format!("{} ({id})", option.label),
            _ => option.label.clone(),
        };
        let base = label.clone();
        let mut n = 2;
        while !used.insert(label.clone()) {
            label = format!("{base} ({n})");
            n += 1;
        }
        option.label = label;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PreviousWorkflow, WorkflowOption, WorkflowView, WorkflowViewKind, build_workflow_view,
        missing_settings_workflow, offered, preselected_index, shows_settings_workflow_hint,
        workflow_choices, workflow_options,
    };
    use crate::commands::{CommitWorkflows, WorkflowData, WorkflowInfo, WorkflowIntent};

    fn wf(id: &str, name: Option<&str>) -> WorkflowInfo {
        WorkflowInfo {
            id: id.to_string(),
            name: name.map(str::to_string),
            description: None,
            metadata_schema_url: None,
            entries_schema_url: None,
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
            config_url: None,
        };
        let view = build_workflow_view(&workflows, Some("beta"), None);
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
        let view = build_workflow_view(&CommitWorkflows::NotConfigured, None, None);
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
        let view = build_workflow_view(&CommitWorkflows::Unavailable, Some("ignored"), None);
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
                config_url: Some("https://cat/b/b/tree/.quilt/workflows/config.yml".to_string()),
            },
            Some("ignored"),
            None,
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

    // ── First push (the bucket form): no previous → bucket default ──

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
    fn view_settings_workflow_comes_first() {
        // The publish settings' default workflow wins over both the bucket
        // default and the previous pick, as one-click Publish would send it.
        let workflows = CommitWorkflows::Available {
            workflows: vec![
                wf("alpha", Some("Alpha WF")),
                wf("beta", None),
                wf("gamma", None),
            ],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: true,
            config_url: None,
        };
        let view = build_workflow_view(&workflows, Some("beta"), Some("gamma"));
        assert_eq!(
            view.options[view.initial].intent,
            WorkflowIntent::Named("gamma".to_string())
        );
    }

    #[test]
    fn view_settings_workflow_the_bucket_does_not_declare_falls_back() {
        // A settings workflow this bucket doesn't declare leaves the bucket's
        // own preselection rules unchanged.
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: false,
            config_url: None,
        };
        let view = build_workflow_view(&workflows, Some("beta"), Some("ghost"));
        assert_eq!(
            view.initial,
            preselected_index(
                &[wf("alpha", Some("Alpha WF")), wf("beta", None)],
                Some("beta"),
                Some("alpha"),
                false
            )
        );
        // Required, no bucket default: the previous pick still breaks the tie.
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: None,
            is_workflow_required: true,
            config_url: None,
        };
        let view = build_workflow_view(&workflows, Some("beta"), Some("ghost"));
        assert_eq!(view.initial, 2);
    }

    #[test]
    fn view_settings_workflow_is_ignored_without_a_config() {
        for state in [CommitWorkflows::NotConfigured, CommitWorkflows::Unavailable] {
            let view = build_workflow_view(&state, None, Some("gamma"));
            assert_eq!(view.initial, 0);
            assert_eq!(view.options[0].intent, WorkflowIntent::BucketDefault);
        }
    }

    #[test]
    fn settings_workflow_missing_from_the_bucket_is_reported() {
        let available = build_workflow_view(
            &CommitWorkflows::Available {
                workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
                default_workflow: None,
                is_workflow_required: false,
                config_url: None,
            },
            None,
            Some("wrong-workflow"),
        );
        // Not declared: reported, and the selection falls back to the bucket's.
        assert_eq!(
            missing_settings_workflow(&available, Some("wrong-workflow")),
            Some("wrong-workflow")
        );
        assert_eq!(available.initial, 0);
        // Declared: nothing to report.
        assert_eq!(missing_settings_workflow(&available, Some("beta")), None);
        // No Settings workflow: nothing to report, and no nudge to set one.
        assert_eq!(missing_settings_workflow(&available, None), None);
        // A bucket with no workflows config can't use any named workflow.
        let not_configured = build_workflow_view(&CommitWorkflows::NotConfigured, None, None);
        assert_eq!(
            missing_settings_workflow(&not_configured, Some("beta")),
            Some("beta")
        );
        // Couldn't load the config: unknown, so no claim either way.
        let unavailable = build_workflow_view(&CommitWorkflows::Unavailable, None, None);
        assert_eq!(missing_settings_workflow(&unavailable, Some("beta")), None);
    }

    #[test]
    fn settings_workflow_hint_follows_the_selection() {
        let gamma = WorkflowIntent::Named("gamma".to_string());
        // Shown while the settings' workflow is the one selected.
        assert!(shows_settings_workflow_hint(Some(&gamma), Some("gamma")));
        // Hidden once the user picks something else.
        assert!(!shows_settings_workflow_hint(
            Some(&WorkflowIntent::Named("alpha".to_string())),
            Some("gamma")
        ));
        assert!(!shows_settings_workflow_hint(
            Some(&WorkflowIntent::NoWorkflow),
            Some("gamma")
        ));
        // Never shown when Settings names no workflow: no nudge to set one.
        assert!(!shows_settings_workflow_hint(Some(&gamma), None));
    }

    #[test]
    fn view_first_push_available_preselects_bucket_default() {
        // A first push feeds `None` for the previous workflow id; the resulting
        // selection is the bucket default, never `Named("")`.
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: Some("beta".to_string()),
            is_workflow_required: true,
            config_url: None,
        };
        let view = build_workflow_view(&workflows, None, None);
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
            let view = build_workflow_view(&state, None, None);
            assert_eq!(
                view.options[view.initial].intent,
                WorkflowIntent::BucketDefault
            );
        }
    }

    // ── The select ──

    fn option(label: &str, intent: WorkflowIntent, disabled: bool) -> WorkflowOption {
        WorkflowOption {
            label: label.to_string(),
            intent,
            disabled,
        }
    }

    fn named(id: &str) -> WorkflowIntent {
        WorkflowIntent::Named(id.to_string())
    }

    fn available(options: Vec<WorkflowOption>, initial: usize) -> WorkflowView {
        WorkflowView {
            kind: WorkflowViewKind::Available {
                is_workflow_required: true,
            },
            options,
            initial,
        }
    }

    /// A required bucket with one workflow and no default starts unchosen,
    /// as with several: the form does not choose the one for the reader.
    #[test]
    fn the_only_workflow_of_a_required_bucket_must_be_chosen() {
        let (options, initial, needs_choice) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, true),
                option("Plate reads", named("plates"), false),
            ],
            0,
        ));
        assert_eq!(initial, WorkflowIntent::NoWorkflow);
        assert!(needs_choice);
        let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["None", "Plate reads"]);
    }

    /// The unchosen start is offered until the reader chooses, then it goes.
    #[test]
    fn the_unchosen_start_leaves_once_chosen() {
        let (options, _, needs_choice) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, true),
                option("Plate reads", named("plates"), false),
            ],
            0,
        ));
        assert_eq!(
            offered(&options, needs_choice, false),
            ["None", "Plate reads"]
        );
        assert_eq!(offered(&options, needs_choice, true), ["Plate reads"]);
        // With nothing to choose first, `None` is an ordinary option.
        let (options, _, needs_choice) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, false),
                option("Plate reads", named("plates"), false),
            ],
            1,
        ));
        assert_eq!(
            offered(&options, needs_choice, true),
            ["None", "Plate reads"]
        );
    }

    /// With several to choose from, the greyed start stays and blocks.
    #[test]
    fn several_workflows_of_a_required_bucket_must_be_chosen() {
        let (options, initial, needs_choice) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, true),
                option("Plate reads", named("plates"), false),
                option("Exports", named("exports"), false),
            ],
            0,
        ));
        assert_eq!(initial, WorkflowIntent::NoWorkflow);
        assert!(needs_choice);
        let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["None", "Plate reads", "Exports"]);
    }

    /// Two workflows with one name stay two choices, each its own id.
    #[test]
    fn workflows_sharing_a_name_stay_apart() {
        let (options, initial, _) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, false),
                option("Plates", named("plates-a"), false),
                option("Plates", named("plates-b"), false),
            ],
            2,
        ));
        assert_eq!(initial, named("plates-b"));
        let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["None", "Plates (plates-a)", "Plates (plates-b)"]);
    }

    /// A workflow whose own name reads like a told-apart one cannot take
    /// another's label: every label names one intent.
    #[test]
    fn no_two_options_share_a_label() {
        let (options, _, _) = workflow_choices(&available(
            vec![
                option("None", WorkflowIntent::NoWorkflow, false),
                option("Plates (plates-a)", named("odd"), false),
                option("Plates", named("plates-a"), false),
                option("Plates", named("plates-a-2"), false),
                option("None", named("none"), false),
            ],
            0,
        ));
        let labels: Vec<_> = options.iter().map(|o| o.label.clone()).collect();
        let mut distinct = labels.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), labels.len(), "{labels:?}");
        let form_intents: Vec<_> = options.iter().map(|o| o.intent.clone()).collect();
        assert_eq!(
            form_intents,
            [
                WorkflowIntent::NoWorkflow,
                named("odd"),
                named("plates-a"),
                named("plates-a-2"),
                named("none")
            ]
        );
    }
}

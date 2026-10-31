use quilt_uri::{Host, Namespace, S3PackageUri};
use serde::{Deserialize, Serialize};

use crate::engine;

// ── Response types ──────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct InstalledPackageData {
    pub namespace: Namespace,
    pub uri: Option<S3PackageUri>,
    pub status: String,
    /// Hash of the revision currently installed locally, if any. Feeds the
    /// version-mismatch deep-link banner on `pages/installed_package.rs`.
    pub installed_hash: Option<String>,
    /// Commit message of the revision currently installed locally, if any.
    /// See `installed_hash`.
    pub installed_message: Option<String>,
    /// Package has been pushed — the remote is pinned to its push history
    /// and can't be edited. The toolbar's remote button becomes a read-only
    /// "Show remote" view.
    pub remote_locked: bool,
    /// Package has a local commit. Setting a remote only re-commits (creating
    /// a new revision) when there is one, so the Set-remote notice is gated
    /// on this.
    pub has_local_commit: bool,
    /// The active role cannot reach this package's bucket, worded as the
    /// roster words it. The status banner states this instead of the
    /// "unable to check remote status" copy, and must not offer Login: the
    /// session is healthy, so signing in again re-vends the same role.
    pub no_access_reason: Option<String>,
    /// Sorted by path, then capped at 1000.
    pub entries: Vec<EntryData>,
    /// Whole-package facet counts for the v2 file pane.
    pub counts: EntryCounts,
    /// Every entry the package has, ignored ones included, before the cap.
    pub total: usize,
    /// The cap dropped entries. Set by the backend; never infer it from
    /// `entries.len()`.
    pub truncated: bool,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
    /// This package's standing sync scope.
    pub syncs_entire_package: bool,
    /// The scope keeps chosen folders, which neither of this screen's scopes is.
    pub keeps_folders: bool,
    /// Whether the experiment that offers the scope control is on.
    pub entire_package_sync_enabled: bool,
}

/// The v2 file pane's facet counts, over the whole package. The facets are
/// disjoint apart from `all`, which excludes only the ignored entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryCounts {
    pub all: usize,
    /// Added, modified and deleted.
    pub changed: usize,
    pub not_downloaded: usize,
    /// Ignored files and folders.
    pub ignored: usize,
    /// Also inside `ignored`: the folders among them.
    #[serde(default)]
    pub ignored_folders: usize,
    /// Also inside `changed`; on its own because the v2 pane's all-downloaded
    /// caption must know about a deleted file past the list cap.
    pub deleted: usize,
}

/// The rule that ignored an entry: its line as written, and where it came from.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct IgnoredBy {
    pub pattern: String,
    pub source: RuleSource,
}

impl IgnoredBy {
    #[must_use]
    pub fn new(pattern: impl Into<String>, source: RuleSource) -> Self {
        Self {
            pattern: pattern.into(),
            source,
        }
    }

    /// The rule and its source, as an ignored row names them:
    /// `*.tmp · Your rules, line 4`.
    #[must_use]
    pub fn words(&self) -> String {
        format!("{} · {}", self.pattern, self.source.words())
    }
}

/// Where an ignore rule came from. Lines are 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleSource {
    Default,
    /// The user-wide rules file, `$QUILT_DOMAIN/.quiltignore`.
    User {
        line: u64,
    },
    /// The package's own `.quiltignore`.
    Package {
        line: u64,
    },
}

impl RuleSource {
    #[must_use]
    pub fn words(self) -> String {
        match self {
            Self::Default => "Built-in rule".to_string(),
            Self::User { line } => format!("Your rules, line {line}"),
            Self::Package { line } => format!("This package's .quiltignore, line {line}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<IgnoredBy>,
    /// An ignored folder rather than a file: `filename` has no trailing slash
    /// and `size` is 0, because nothing inside it was walked.
    #[serde(default)]
    pub folder: bool,
    pub namespace: Namespace,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: Namespace,
    pub uri: Option<S3PackageUri>,
    /// The package has been pushed, so its remote is pinned to its push
    /// history and can only be shown.
    pub remote_locked: bool,
    /// The remote's `latest` hash, when the read reached the remote; picks
    /// the header's catalog target as [`PackageHeaderData::newer`] does.
    pub newer: Option<String>,
    pub status: String,
    pub message: String,
    /// The current revision's metadata: what an emptied editor keeps.
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    /// The publish settings' default metadata, which the editor starts with
    /// in place of `user_meta`; `None` when the settings have none.
    pub settings_user_meta: Option<String>,
    /// The publish settings' default workflow id, preselected first when the
    /// bucket declares it.
    pub settings_workflow: Option<String>,
    /// Whether the publish settings set a message template.
    pub has_message_template: bool,
    /// The previous revision's stamped workflow selection (its `id`), if any.
    pub workflow: Option<WorkflowData>,
    /// The bucket's workflow-selection situation for the commit dialog.
    pub workflows: CommitWorkflows,
    /// The active role cannot reach this package's bucket, worded as the
    /// roster words it — the same field name, shape and wording as
    /// [`InstalledPackageData::no_access_reason`]. Committing is not offline
    /// work (the workflow gate reads the bucket's config first), so this
    /// disables the commit affordances and supplies their tooltip.
    pub no_access_reason: Option<String>,
    /// There is no session to commit with. Disables the commit affordances
    /// like a denial, but the remedy is a sign-in.
    pub no_session: bool,
    /// The deployment to sign in to, when there is one; `None` for ambient
    /// AWS credentials, fixed in `~/.aws` or by `aws sso login`.
    pub no_session_host: Option<String>,
    pub entries: Vec<EntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    /// The entries by kind over the whole package, not the capped `entries`.
    pub counts: EntryCounts,
    /// The new and changed files' bytes over the whole package; a deletion
    /// adds none.
    pub changed_bytes: u64,
    /// The system files the revision would publish, over the whole package.
    pub junk: Option<JunkSummary>,
}

/// The system files among the changes, uncapped.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkSummary {
    pub count: usize,
    /// The first few distinct file names, in path order.
    pub names: Vec<String>,
    /// How many more distinct names there are.
    pub more_names: usize,
    /// The first one and its pattern, which *Ignore them* offers.
    pub first_path: String,
    pub first_pattern: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowData {
    pub id: Option<String>,
}

/// A workflow declared under `workflows:` in the bucket's config, surfaced to
/// the commit dialog so the user can pick one. UI-side mirror of the backend
/// `CommitWorkflowInfo`. Distinct from [`WorkflowData`], which is the previous
/// revision's stamped selection.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Catalog HTTPS link to the workflow's declared metadata schema object,
    /// pre-formatted by the backend. `None` when the workflow declares no
    /// metadata schema (or there is no catalog host to link against).
    pub metadata_schema_url: Option<String>,
    /// Catalog HTTPS link to the workflow's declared entries schema object.
    pub entries_schema_url: Option<String>,
}

/// The bucket's workflow-selection situation, as sent by the backend
/// (`quilt_sync::commands::commit_data::CommitWorkflows`). The serde
/// attributes MUST stay identical to the backend so the tagged JSON crosses
/// the engine boundary unchanged. Splits the three cases the commit dialog
/// renders distinctly:
/// - `Available` — the bucket has a config; offer its workflow choices.
/// - `NotConfigured` — the bucket is ungoverned; no choice to make.
/// - `Unavailable` — a transient failure loading the config; commit will retry
///   the bucket default.
/// - `Invalid` — the config is malformed; commits will fail until it is fixed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommitWorkflows {
    Available {
        workflows: Vec<WorkflowInfo>,
        default_workflow: Option<String>,
        is_workflow_required: bool,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, pre-formatted by the backend. `None` when there is no
        /// catalog host to link against.
        config_url: Option<String>,
    },
    NotConfigured,
    Unavailable,
    Invalid {
        reason: String,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, pre-formatted by the backend. `None` when there is no
        /// catalog host to link against.
        config_url: Option<String>,
    },
}

/// Which commit-dialog input a [`CommitViolation`] belongs under, so the UI can
/// render each advisory violation beneath the field the user must fix. UI-side
/// mirror of the backend `quilt_sync::commands::commit_data::ViolationField`;
/// the serde attributes MUST match so the tagged JSON crosses the engine boundary
/// unchanged.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ViolationField {
    Message,
    Metadata,
    Name,
}

/// A single advisory workflow violation for the commit dialog. UI-side mirror of
/// the backend `CommitViolation`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitViolation {
    pub field: ViolationField,
    pub message: String,
}

/// Caller intent for resolving a package's workflow, sent with a commit.
///
/// UI-side mirror of `quilt_rs::io::remote::WorkflowIntent`. The serde
/// attributes MUST stay identical to the backend so the tagged JSON crosses
/// the engine boundary unchanged:
/// - `{"kind":"bucket-default"}` — no opinion; honour the bucket default.
/// - `{"kind":"no-workflow"}` — explicit opt-out.
/// - `{"kind":"named","id":"x"}` — an exact workflow id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum WorkflowIntent {
    BucketDefault,
    NoWorkflow,
    Named(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: Namespace,
    pub uri: Option<S3PackageUri>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub host: String,
    pub back: String,
    pub catalog_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorData {
    pub title: String,
    pub message: String,
    pub login_host: String,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishSettingsData {
    pub message_template: String,
    pub default_workflow: String,
    pub default_metadata: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutosyncSettingsData {
    pub pull_enabled: bool,
    pub push_enabled: bool,
    pub pull_interval_secs: u64,
    pub idle_timeout_secs: u64,
    pub close_to_tray: bool,
}

impl Default for AutosyncSettingsData {
    fn default() -> Self {
        Self {
            pull_enabled: false,
            push_enabled: false,
            pull_interval_secs: 60,
            idle_timeout_secs: 300,
            close_to_tray: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsWatcherSettingsData {
    pub enabled: bool,
}

impl Default for FsWatcherSettingsData {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Opt-ins for behaviour that is not finished being designed. Everything here
/// is off unless the user went looking for it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalSettingsData {
    pub entire_package_sync: bool,
    /// Whether the v2 Settings page shows the Agent tasks card.
    #[serde(default)]
    pub agent_tasks: bool,
}

/// `task_settings.json`: which buckets may hand this computer a task, and the
/// agents it can run. Field names as the file has them.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TaskSettings {
    #[serde(default)]
    pub blessed_buckets: Vec<String>,
    #[serde(default)]
    pub agents: std::collections::BTreeMap<String, AgentSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentSettings {
    /// The program, then its arguments.
    pub command: Vec<String>,
    /// Approve every permission request the agent makes.
    #[serde(default)]
    pub trust_all: bool,
    /// Variables set for the agent. Settings keeps them as they are.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// `task_settings.json` as read.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSettingsData {
    /// Empty when there is no file, or it could not be read.
    pub settings: TaskSettings,
    /// Why the file could not be read.
    pub error: Option<String>,
}

/// Which design generation the app draws: v2 unless the reader chose the
/// classic design.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Design {
    V1,
    #[default]
    V2,
}

/// One built-in ignore rule, its place in the popup, and whether it is on.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DefaultRuleData {
    pub rule: String,
    pub group: String,
    pub about: String,
    pub on: bool,
}

/// The built-in ignore rules, and the user-wide file that turns them off.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRulesData {
    pub file: String,
    /// `None` when the file could not be read.
    pub rules: Option<Vec<DefaultRuleData>>,
}

/// What `QUILT_LOG` does to the saved log level.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum LogEnv {
    Unset,
    /// Valid, so it replaces the saved choice.
    Overrides(String),
    /// Invalid, so ignored.
    Ignored(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub version: String,
    pub home_dir: Option<String>,
    pub data_dir: String,
    pub auth_hosts: Vec<String>,
    /// The saved log level, lowercase (`default`, `trace`, …).
    pub log_level: String,
    pub log_env: LogEnv,
    pub logs_dir: String,
    pub logs_dir_is_temporary: bool,
    pub os: String,
    pub changelog: Vec<ChangelogEntry>,
    pub publish: PublishSettingsData,
    pub autosync: AutosyncSettingsData,
    pub fswatcher: FsWatcherSettingsData,
    pub experimental: ExperimentalSettingsData,
    #[serde(default)]
    pub design: Design,
    #[serde(default)]
    pub ignore_rules: IgnoreRulesData,
    #[serde(default)]
    pub tasks: TaskSettingsData,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupData {
    pub default_home: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<PackageItemData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemData {
    pub namespace: Namespace,
    pub status: String,
    pub has_changes: bool,
    /// Package has a local commit. Setting a remote only re-commits (creating
    /// a new revision) when there is one, so the Set-remote notice is gated
    /// on this.
    pub has_local_commit: bool,
    pub uri: Option<S3PackageUri>,
    pub remote_display: Option<String>,
    /// The autosync watcher's `Other` pause message for this namespace,
    /// fetched with the list data. `Some` means the row is autosync-paused
    /// for a reason the status string cannot carry, so it renders red with
    /// this reason as its third line; `None` means no such pause. Sourced
    /// directly from the backend's authoritative paused map — there is no
    /// frontend cache to go stale.
    pub paused_reason: Option<String>,
    /// The stable reason discriminant for the pause (`"pullConflict"`,
    /// `"other"`, …), paired with `paused_reason`. `Some` exactly when
    /// `paused_reason` is; lets the row pick conflict- vs. generic guidance
    /// without re-parsing the message.
    pub paused_kind: Option<String>,
    /// The active role cannot reach this row's bucket. The row explains
    /// itself with `no_access_reason` and must **not** offer Login: the
    /// session is fine, so signing in again re-vends the same denied role.
    pub no_access: bool,
    /// Why the row is marked, naming the active role. `Some` exactly when
    /// `no_access` is.
    pub no_access_reason: Option<String>,
    /// The host whose role selector the switch affordance opens. `Some`
    /// only when the user holds more than one role there, so a single-role
    /// user never gets a dead-end button.
    pub role_switch_host: Option<String>,
}

/// A deep-link banner to surface on the installed-package page after
/// resolving a `quilt+s3://` remote package URI. UI-side mirror of the
/// backend `quilt_sync::commands::remote_package::RemoteBanner`; the serde
/// attributes MUST match so the tagged JSON crosses the engine boundary
/// unchanged:
/// - `differentVersion` — the requested revision differs from what's
///   installed locally.
/// - `localOnly` — the package has no remote; it's local-only.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RemoteBanner {
    DifferentVersion {
        requested_hash: String,
        requested_bucket: String,
        requested_origin: Option<Host>,
        installed_hash: String,
    },
    LocalOnly,
}

/// The v2 package page's payload. Mirrors
/// `src-tauri/src/engine/commands/package_page.rs` field for field.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagePageData {
    pub header: PackageHeaderData,
    pub context: PackageContextData,
    /// Why autosync stopped, when the reason is one no state covers. `None` for
    /// every other pause, because those resolve into `header.state`.
    pub sync_paused: Option<String>,
    /// The file pane's list, classified by the header's own status.
    pub files: FilesData,
}

/// The package's entries, sorted by path and capped, with the whole-package
/// facts the cap would otherwise hide.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryList {
    pub entries: Vec<EntryData>,
    pub counts: EntryCounts,
    /// Every entry, ignored ones included, before the cap.
    pub total: usize,
    /// The cap dropped entries. Never inferred from `entries.len()`.
    pub truncated: bool,
}

/// The file pane's list, or why there is none. A blocked remote status still
/// lists the files from a local one; no list is sent only when not even that
/// could be computed, rather than one classified without a status.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FilesData {
    Listed(EntryList),
    Unlisted { reason: String },
}

/// The read-only facts shown beside the v2 package page.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackageContextData {
    pub revision: CurrentRevisionData,
    pub bucket: Option<String>,
    /// How many revisions this copy holds — the trigger's N. The list itself
    /// is `get_revision_history`, fetched on open.
    pub revision_count: usize,
    pub keeping: KeepingData,
    /// The current revision's bytes; `None` when they cannot be read, and
    /// Keeping's caption then gives the count alone.
    pub size: Option<PackageSize>,
    /// The resolve comparison; `Some` only while the package is diverged.
    pub resolve: Option<ResolveData>,
}

/// The current revision's size, from its manifest rows: logical bytes, not
/// disk usage. Mirrors `src-tauri/src/engine/commands/package_page.rs`; the serde
/// attributes MUST match.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageSize {
    /// Every listed file.
    pub total: u64,
    /// Every listed file not outstanding, a file deleted here included.
    pub downloaded: u64,
    /// The backlog's files: what `Download N files` fetches.
    pub backlog: u64,
}

/// What each resolve choice would cost, or why that cannot be read.
/// Mirrors `src-tauri/src/engine/commands/package_page.rs`; the serde attributes MUST match.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ResolveData {
    Compared {
        published_message: Option<String>,
        /// Logical keys that differ, sorted and uncapped: the file pane's marks.
        differing: Vec<String>,
        unpublished: usize,
        uncommitted: usize,
    },
    Refused {
        reason: String,
    },
}

/// Which files this copy keeps. Mirrors
/// `src-tauri/src/engine/commands/package_page.rs`; the serde attributes MUST match.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeepingScope {
    IndividualFiles,
    EntirePackage,
    /// Every file under some folders.
    Folders,
}

/// The Keeping section: the standing rule, and what it has not yet fetched.
/// Mirrors `src-tauri/src/engine/commands/package_page.rs` field for field.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeepingData {
    pub scope: KeepingScope,
    /// The kept folders under folders scope, which the caption names.
    pub folders: Vec<String>,
    /// Files in the current revision — the caption's M.
    pub total: usize,
    /// Listed files not downloaded here, so the caption's M less its N.
    pub outstanding: usize,
    /// The backlog, and exactly what `Download N files` installs. Empty under
    /// individual-file scope.
    pub remote_only: Vec<String>,
    /// Files deleted here, which count as downloaded and the caption names;
    /// 0 when the status could not be read.
    pub deleted_here: usize,
}

/// The current revision's user-facing facts.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRevisionData {
    /// Top hash of the revision's manifest: the mismatch band's hover text.
    pub hash: String,
    pub message: Option<String>,
    pub obtained_at: f64,
}

/// `History`: the rows, and the footer's figure. Mirrors
/// `src-tauri/src/engine/commands/package_page.rs` field for field.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevisionHistoryData {
    pub rows: Vec<RevisionHistoryRow>,
    /// What removing every removable row frees, measured for the set. `None`
    /// when nothing is removable or the measure could not be read.
    pub removable_frees: Option<u64>,
}

/// Why a revision is kept. Mirrors `KeptReason` in
/// `src-tauri/src/engine/commands/package_page.rs`; the serde attributes MUST match.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeptReason {
    Current,
    Latest,
    Base,
    NotPushed,
    Unpublished,
}

/// One row of `History`. Mirrors
/// `src-tauri/src/engine/commands/package_page.rs` field for field.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevisionHistoryRow {
    /// What `remove_revisions` takes. Never drawn.
    pub hash: String,
    pub message: Option<String>,
    pub obtained_at: f64,
    pub published: bool,
    /// The catalog page for exactly this revision; `None` when unpublished or
    /// when the remote has no catalog host.
    pub catalog_url: Option<String>,
    /// Why it cannot be removed; empty when it can.
    pub kept: Vec<KeptReason>,
    /// What removing it alone frees; `None` when kept or unmeasured.
    pub frees: Option<u64>,
}

/// The header region: identity, one resolved condition, and what the overflow
/// menu may offer.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageHeaderData {
    pub namespace: Namespace,
    /// Carried whole rather than as a catalog URL: the menu builds an
    /// entry-level link from the same value, and `util::catalog_url` owns that
    /// formatting.
    pub uri: Option<S3PackageUri>,
    pub state: crate::kit::PackageState,
    /// Whether the remote is pinned by a push — decides whether the menu offers
    /// to change the bucket or only to show it.
    pub remote_locked: bool,
    /// Whether there is a pending commit. **Not "undo is available"** — see
    /// `commit_has_parent`.
    pub has_local_commit: bool,
    /// Whether that pending commit has a revision behind it. Undo's floor.
    ///
    /// Undo is bounded by three facts and the menu must compose all of them:
    /// `has_local_commit`, this, and `uri.is_none()` — the engine refuses on any
    /// remote, because a push consumes the commit chain. The words for each
    /// refusal live here, not on the wire.
    pub commit_has_parent: bool,
    pub role_switch: Option<RoleSwitch>,
    /// The newer revision this read saw, when it reached the remote; the key
    /// the page's kept dry-run answer must match.
    pub newer: Option<String>,
}

/// The remedy a denial offers. UI-side mirror of
/// `quilt_sync::commands::package_page::RoleSwitch`; the serde attributes MUST
/// match so the payload crosses the engine boundary unchanged.
///
/// A sibling of `state` rather than a field of `PackageState::RoleDenied`: the
/// kit's state enum is the vocabulary, and a host and a role list are transport.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleSwitch {
    pub host: String,
    pub alternatives: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: Namespace,
    pub banner: Option<RemoteBanner>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// The download for the engine's machine, for an engine that can't
    /// install it itself.
    #[serde(default)]
    pub url: Option<String>,
}

// ── Data-fetching commands ──────────────────────────────────

pub async fn get_installed_package_data(
    namespace: String,
    filter: Option<String>,
) -> Result<InstalledPackageData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        filter: Option<String>,
    }
    engine::invoke("get_installed_package_data", &Args { namespace, filter }).await
}

/// Read everything the v2 package page draws, for one package.
///
/// One read for the page rather than one per region — see
/// `src-tauri/src/engine/commands/package_page.rs`.
pub async fn get_package_page_data(namespace: String) -> Result<PackagePageData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("get_package_page_data", &Args { namespace }).await
}

/// The revisions this copy holds, newest obtained first. Called when the
/// context pane's popover opens, never on page load.
pub async fn get_revision_history(namespace: String) -> Result<RevisionHistoryData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("get_revision_history", &Args { namespace }).await
}

/// Remove these old revisions of an installed package, and the objects only
/// they used. The backend posts the result to the notification stack; the
/// answer is empty on success, and the refusal's sentence otherwise.
pub async fn remove_revisions(namespace: String, hashes: Vec<String>) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        hashes: Vec<String>,
    }
    engine::invoke("remove_revisions", &Args { namespace, hashes }).await
}

/// The paths a download left out. Mirrors
/// `src-tauri/src/engine/commands/package_page.rs` field for field.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotDownloaded {
    /// Their bytes are no longer in the bucket at this revision.
    pub skipped: Vec<String>,
    /// Their bytes do not match the hash the package records.
    pub hash_mismatch: Vec<String>,
}

/// Install these paths of an installed package: Keeping's backlog, and the
/// file pane's `[Download]`.
///
/// Answers with the paths it left out; both lists empty when all came down.
pub async fn package_download_backlog(
    namespace: String,
    paths: Vec<String>,
) -> Result<NotDownloaded, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        paths: Vec<String>,
    }
    engine::invoke("package_download_backlog", &Args { namespace, paths }).await
}

pub async fn get_commit_data(namespace: String) -> Result<CommitData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("get_commit_data", &Args { namespace }).await
}

/// Fetch and cache the selected workflow's rules for live commit-dialog
/// validation. Call when the workflow selection changes; the fetch runs once per
/// `(namespace, workflow)` and later calls hit the backend cache. Returns
/// whether the workflow has rules to validate against.
///
/// Pass `refresh = true` on the dialog's first load of a session so the backend
/// drops the namespace's cached entries and re-fetches — the cache is
/// app-lifetime state, so this is how a config.yml change since the last open is
/// picked up. Later loads within the session pass `false`.
pub async fn load_workflow_rules(
    namespace: String,
    workflow_id: String,
    refresh: bool,
) -> Result<bool, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        workflow_id: String,
        refresh: bool,
    }
    engine::invoke(
        "load_workflow_rules",
        &Args {
            namespace,
            workflow_id,
            refresh,
        },
    )
    .await
}

/// Validate the current commit-dialog input against the cached rules for the
/// selected workflow. Pure cache read on the backend — no network I/O — so it is
/// safe to call on every (debounced) keystroke. Returns advisory violations
/// routed per field; empty means the input satisfies the workflow.
pub async fn validate_commit_candidate(
    namespace: String,
    workflow_id: String,
    message: String,
    user_meta: String,
    name: String,
) -> Result<Vec<CommitViolation>, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        workflow_id: String,
        message: String,
        user_meta: String,
        name: String,
    }
    engine::invoke(
        "validate_commit_candidate",
        &Args {
            namespace,
            workflow_id,
            message,
            user_meta,
            name,
        },
    )
    .await
}

pub async fn get_merge_data(namespace: String) -> Result<MergeData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("get_merge_data", &Args { namespace }).await
}

pub async fn get_login_data(host: String, back: String) -> Result<LoginData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
        back: String,
    }
    engine::invoke("get_login_data", &Args { host, back }).await
}

pub async fn get_login_error_data(
    host: String,
    title: Option<String>,
    error: String,
) -> Result<LoginErrorData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
        title: Option<String>,
        error: String,
    }
    engine::invoke("get_login_error_data", &Args { host, title, error }).await
}
/// Put `text` on the system clipboard.
///
/// `uri` is the package the text addresses, for the backend's event only — see
/// `commands.rs`'s own note on the `uri` argument several commands take and
/// never use.
pub async fn copy_to_clipboard(text: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    use leptos::prelude::GetUntracked;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        text: String,
        uri: Option<S3PackageUri>,
    }
    // Headless has no desktop clipboard; the reader's browser has one. Only on
    // a secure origin, which loopback is.
    if engine::headless().get_untracked() {
        let clipboard = web_sys::window()
            .ok_or("No window")?
            .navigator()
            .clipboard();
        wasm_bindgen_futures::JsFuture::from(clipboard.write_text(&text))
            .await
            .map_err(|_| "This browser didn't allow copying here".to_owned())?;
        return Ok("Copied to the clipboard".to_owned());
    }
    engine::invoke("copy_to_clipboard", &Args { text, uri }).await
}

/// Where one of a package's files is on disk, resolved as opening it resolves
/// it. For the file pane's *Copy path*.
pub async fn package_file_path(namespace: String, path: String) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        path: String,
    }
    engine::invoke("package_file_path", &Args { namespace, path }).await
}

pub async fn get_settings_data() -> Result<SettingsData, String> {
    engine::invoke_unit("get_settings_data").await
}

/// Whether a typed path is there, and a folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathCheck {
    pub exists: bool,
    pub is_dir: bool,
}

pub async fn check_path(path: String) -> Result<PathCheck, String> {
    #[derive(Serialize)]
    struct Args {
        path: String,
    }
    engine::invoke("check_path", &Args { path }).await
}

/// The folders whose path starts with `prefix`, each ending in a separator.
pub async fn complete_path(prefix: String) -> Result<Vec<String>, String> {
    #[derive(Serialize)]
    struct Args {
        prefix: String,
    }
    engine::invoke("complete_path", &Args { prefix }).await
}

pub async fn get_setup_data() -> Result<SetupData, String> {
    engine::invoke_unit("get_setup_data").await
}

pub async fn get_installed_packages_list_data() -> Result<InstalledPackagesListData, String> {
    engine::invoke_unit("get_installed_packages_list_data").await
}

/// v2's package list, light phase. Every row arrives `provisional`.
pub async fn get_main_page_packages() -> Result<MainPagePackagesData, String> {
    engine::invoke_unit("get_main_page_packages").await
}

/// v2's package list, heavy phase. One invocation per row, fired by the
/// page's resolve; `pages::main_page::record_refresh` applies the answer.
pub async fn refresh_main_page_package(
    namespace: String,
) -> Result<MainPagePackageRefreshData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("refresh_main_page_package", &Args { namespace }).await
}

/// What the heavy phase corrects on a row. Mirrors `MainPagePackageRefresh` on
/// the engine's side; the fields the light phase already delivered are not resent.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackageRefreshData {
    pub state: crate::kit::PackageState,
    pub role_switch_host: Option<String>,
    /// `None` when the status check did not run or failed.
    pub downloads: Option<MainPageDownloadsData>,
    /// Files changed here since the last commit; `None` when the status check
    /// did not run or failed. Apart from `state`, which can hide it.
    pub changed_files: Option<usize>,
}

/// What a copy holds of its current revision. Mirrors `MainPageDownloads` on
/// the engine's side.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MainPageDownloadsData {
    pub scope: KeepingScope,
    /// How many folders a folders scope keeps; 0 for any other scope.
    pub folders: usize,
    /// The files the current revision lists.
    pub total: usize,
    /// The listed files not downloaded here.
    pub outstanding: usize,
    /// The outstanding files the scope covers, which Keeping downloads.
    pub backlog: usize,
    /// Every deletion the status reports.
    pub deleted_here: usize,
    /// The listed files' bytes; `None` when they cannot be read.
    pub size: Option<PackageSize>,
}

/// Keeping's counts, so the row's tooltip says what the package page says.
impl From<MainPageDownloadsData> for crate::util::Held {
    fn from(data: MainPageDownloadsData) -> Self {
        Self {
            total: data.total,
            outstanding: data.outstanding,
            deleted_here: data.deleted_here,
            size: data.size,
        }
    }
}

impl From<MainPageDownloadsData> for crate::kit::RowDownloads {
    fn from(data: MainPageDownloadsData) -> Self {
        match data.scope {
            KeepingScope::EntirePackage => Self::EntirePackage {
                to_download: data.backlog,
            },
            KeepingScope::Folders => Self::Folders {
                folders: data.folders,
                to_download: data.backlog,
            },
            KeepingScope::IndividualFiles => Self::IndividualFiles {
                downloaded: data.total.saturating_sub(data.outstanding),
                total: data.total,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackagesData {
    pub packages: Vec<MainPagePackageData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackageData {
    pub namespace: Namespace,
    pub state: crate::kit::PackageState,
    /// Epoch milliseconds, from the last commit or the last installed path — see
    /// `last_changed` on the engine's side. `None` only when nothing has ever been
    /// written to the package.
    pub changed_at: Option<f64>,
    /// The queue's group key for `RoleDenied` (§4.3, R2): one host can hold
    /// both readable and unreadable buckets, so `derive_queue` groups a
    /// denial by this rather than by `host`.
    pub bucket: Option<String>,
    /// The catalog this package points at, as the queue's join key: R3 groups
    /// an `Unknown` package under its host's `Signed out from {host}` cause
    /// only when the accounts payload agrees that host is signed out.
    pub host: Option<String>,
    /// Whether the state is a cached guess awaiting the heavy phase. Almost
    /// always true. `PullConflict` and `Paused` are the exceptions: they come
    /// from the watcher's paused map rather than from cached lineage, so they
    /// arrive settled. `pages::main_page::PackageStore::seed` reads this per row,
    /// so a conflict stays in the queue while the remote is unreachable.
    pub provisional: bool,
    /// The host whose role selector the row's switch affordance opens. The page
    /// carries it into `RowSignals` and settles it on refresh; the switch
    /// control itself is the queue's (Plan 4), so no `#[expect(dead_code)]`
    /// here — the field is carried and settled, never read.
    pub role_switch_host: Option<String>,
}

/// Whether the pull's next check is counting down. Drift is caught by
/// `main_page_watcher_data_wire_form_is_verbatim`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToggleActivityData {
    Armed,
    Idle,
}

/// UI-side mirror of the backend's `ToggleState`, the pull direction.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleStateData {
    pub enabled: bool,
    pub activity: ToggleActivityData,
    /// Epoch milliseconds. `Some` exactly when `activity` is `Armed` — the
    /// backend derives both from one expression.
    pub deadline: Option<f64>,
    pub interval_ms: f64,
}

/// Why the watcher stopped syncing one package. UI-side mirror of the backend's
/// `PausedDto`.
///
/// `Unrecognised` is `#[serde(other)]`: a reason added to the backend without an
/// arm here degrades to one variant instead of failing the whole payload and
/// taking the card with it. It carries no data, because `#[serde(other)]`
/// accepts only unit variants — and does not need to, since the fixed words for
/// an unexplained pause are the UI's anyway.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausedReasonData {
    PendingChanges,
    PendingCommit,
    Diverged,
    PullConflict {
        files: Vec<String>,
    },
    RoleDenied {
        role: Option<String>,
    },
    Other {
        message: String,
    },
    #[serde(other)]
    Unrecognised,
}

/// The Autosync card carries this list but renders nothing from it. The queue
/// (`pages::main_page::queue`) does not read it either: it derives from the
/// resolved package state, never from a pause map. So nothing reads a pause's
/// namespace or reason. A pause that has no state of its own still reaches the
/// queue, because the backend resolves it to `PackageState::Paused`.
///
/// No `dead_code` suppression is needed: this is a library module, and
/// `dead_code` does not flag an unused `pub` item in a library.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedPackageData {
    pub namespace: Namespace,
    pub reason: PausedReasonData,
}

/// One package's autosync state. UI-side mirror of the backend's
/// `AutosyncStateDto`; times are epoch milliseconds. `Unrecognised` lets a
/// state added to the backend degrade instead of failing the payload.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AutosyncStateData {
    Waiting {
        publish_at: f64,
        changed_while_closed: bool,
    },
    Publishing,
    Retrying {
        error: String,
        next_try_at: f64,
    },
    Paused {
        reason: PausedReasonData,
    },
    Off,
    #[serde(other)]
    Unrecognised,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageAutosyncData {
    pub namespace: Namespace,
    pub state: AutosyncStateData,
}

/// The publish direction: the setting, the wait, and each package with
/// something to show. A package that isn't listed has no changes.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishStateData {
    pub enabled: bool,
    pub interval_ms: f64,
    pub packages: Vec<PackageAutosyncData>,
}

/// Payload 3: the watcher's own state. Returned by `get_main_page_watcher`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPageWatcherData {
    pub pull: ToggleStateData,
    pub publish: PublishStateData,
    /// Read only by the wire-form test. The queue reads the resolved package
    /// state, never this pause map. A pause with no state of its own, such as a
    /// workflow rejection, reaches the queue as `PackageState::Paused`, which the
    /// backend resolves for the row.
    pub paused: Vec<PausedPackageData>,
}

/// v2's watcher state: both toggles and the pause list. Drawn by
/// `pages::main_page::autosync::AutosyncCard`.
pub async fn get_main_page_watcher() -> Result<MainPageWatcherData, String> {
    engine::invoke_unit("get_main_page_watcher").await
}

/// A package's autosync state may have changed: read the watcher's payload
/// again. Carries nothing. Kept in lockstep with the backend.
pub const AUTOSYNC_CHANGED_EVENT: &str = "autosync-changed";

/// A package was created, installed or uninstalled, maybe from another window
/// or tab: read the package list again. Carries nothing. Kept in lockstep with
/// the backend.
pub const PACKAGES_CHANGED_EVENT: &str = "packages-changed";

/// The engine sent a page somewhere, after a login or a deep link: each page
/// asks [`pending_navigation`] whether it was this one. Carries nothing. Kept in
/// lockstep with the backend.
pub const NAVIGATE_EVENT: &str = "navigate";

/// One host in the Accounts card, as it arrives.
///
/// Mirrors `quilt_sync::commands::main_page::AccountHost`. `currentRole` is
/// `null` both before the role is resolved and when the query failed — the two
/// are told apart by `provisional`, not by the role.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountHostData {
    pub host: String,
    pub signed_in: bool,
    pub current_role: Option<String>,
    pub roles: Vec<String>,
    pub provisional: bool,
}

/// Payload of `get_main_page_accounts`: the Accounts card's light phase.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MainPageAccountsData {
    pub hosts: Vec<AccountHostData>,
}

/// v2's accounts list, light phase: one row per host, `signedIn` only.
pub async fn get_main_page_accounts() -> Result<MainPageAccountsData, String> {
    engine::invoke_unit("get_main_page_accounts").await
}

/// v2's accounts list, heavy phase: fills in the given host's role.
pub async fn refresh_main_page_account(host: String) -> Result<AccountHostData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
    }
    engine::invoke("refresh_main_page_account", &Args { host }).await
}

/// One installed or published file, flat across every package. Mirrors the
/// backend's `MainPageFile`; the owning package travels with the row rather
/// than grouping it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPageFileData {
    pub path: String,
    pub namespace: Namespace,
    /// Epoch milliseconds. Never re-sorted or re-capped on this side — see
    /// `pages::main_page::recent_files::RecentFilesRegion`.
    pub changed_at: f64,
    /// The revision the package holds, when its remote has it and the package
    /// has no unpublished revision; the catalog link and the copied URI name
    /// it. `None` otherwise. Read from the lineage alone.
    pub published_hash: Option<String>,
}

/// Payload of `get_main_page_recent_files`: §3.2's flat feed, already newest
/// first and already bounded by the backend (§4.5).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPageRecentFilesData {
    pub files: Vec<MainPageFileData>,
}

/// v2's recent files feed, single-phase: drawn by
/// `pages::main_page::recent_files::RecentFilesRegion`, and read by the main
/// page's own resource only once the reader asks for that view.
pub async fn get_main_page_recent_files() -> Result<MainPageRecentFilesData, String> {
    engine::invoke_unit("get_main_page_recent_files").await
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
    /// See [`PackageItemData::no_access`]. Carried here too, so a denial
    /// the heavy phase discovers reaches a row the light phase's bucket
    /// list had cleared.
    pub no_access: bool,
    pub no_access_reason: Option<String>,
    pub role_switch_host: Option<String>,
}

/// Payload of the `package-status-changed` engine event. Same shape as
/// [`RefreshedPackageStatus`] plus the namespace it applies to, so the
/// list/detail pages can match a row by namespace.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatusEvent {
    pub namespace: Namespace,
    pub status: String,
    pub has_changes: bool,
    /// Digest of the observation this event reports (see the backend's
    /// `PackageStatusEvent`). The page acts only when it differs from the last
    /// one it acted on, so a tick that re-reports an unchanged tree — same
    /// fingerprint — leaves the page untouched instead of rebuilding it.
    pub fingerprint: String,
}

pub const PACKAGE_STATUS_EVENT: &str = "package-status-changed";

/// Payload of the `autosync-published` engine event — emitted after a
/// background autosync tick successfully publishes a package. UI listens
/// for this on the installed-packages list page and surfaces it as a
/// toast, mirroring the manual Commit & Push success notification.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedEvent {
    pub namespace: Namespace,
    pub message: String,
}

pub const AUTOSYNC_PUBLISHED_EVENT: &str = "autosync-published";

/// Payload of the `autosync-paused` engine event — emitted when the
/// background watcher pauses a namespace. The `reason` field is a stable
/// string discriminant; `message` is populated only for `reason = "other"`
/// (workflow rejection, hash mismatch, JSON parse failure, etc.) and is
/// what the per-package banner renders so the user knows what to fix.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedEvent {
    pub namespace: Namespace,
    pub reason: String,
    pub message: Option<String>,
}

pub const AUTOSYNC_PAUSED_EVENT: &str = "autosync-paused";

/// Point-in-time view of the autosync watcher's per-namespace state.
/// Returned by `get_autosync_snapshot`; the UI uses it to re-hydrate
/// the paused banner when a page mounts after the watcher already
/// paused a namespace (the `autosync-paused` event alone would miss
/// those pauses).
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WatcherSnapshot {
    pub paused: Vec<PausedEvent>,
}

pub async fn get_autosync_snapshot() -> Result<WatcherSnapshot, String> {
    #[derive(Serialize)]
    struct Args {}
    engine::invoke("get_autosync_snapshot", &Args {}).await
}

pub async fn refresh_package_status(namespace: String) -> Result<RefreshedPackageStatus, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    engine::invoke("refresh_package_status", &Args { namespace }).await
}

pub async fn handle_remote_package(uri: String) -> Result<RemotePackageResult, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        uri: String,
    }
    engine::invoke("handle_remote_package", &Args { uri }).await
}

/// Fetch the commit message for a specific revision of a package, so the
/// installed-package page can show what the requested (but not installed)
/// revision says, alongside the currently installed one.
pub async fn get_revision_message(
    bucket: String,
    namespace: String,
    hash: String,
    catalog: Option<String>,
) -> Result<Option<String>, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        bucket: String,
        namespace: String,
        hash: String,
        catalog: Option<String>,
    }
    engine::invoke(
        "get_revision_message",
        &Args {
            bucket,
            namespace,
            hash,
            catalog,
        },
    )
    .await
}

// ── Auto-update ────────────────────────────────────────────

pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    engine::invoke_unit("check_for_update").await
}

pub async fn download_and_install_update() -> Result<(), String> {
    engine::invoke_unit("download_and_install_update").await
}

// ── Package actions ─────────────────────────────────────────

pub async fn package_commit(
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "package_commit",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
            uri,
        },
    )
    .await
}

pub async fn package_push(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("package_push", &Args { namespace, uri }).await
}

pub async fn package_publish(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("package_publish", &Args { namespace, uri }).await
}

pub async fn package_commit_and_push(
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "package_commit_and_push",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
            uri,
        },
    )
    .await
}

pub async fn update_publish_settings(
    message_template: String,
    default_workflow: String,
    default_metadata: String,
) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        message_template: String,
        default_workflow: String,
        default_metadata: String,
    }
    engine::invoke(
        "update_publish_settings",
        &Args {
            message_template,
            default_workflow,
            default_metadata,
        },
    )
    .await
}

pub async fn update_autosync_settings(settings: AutosyncSettingsData) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        settings: AutosyncSettingsData,
    }
    engine::invoke("update_autosync_settings", &Args { settings }).await
}

/// Some of autosync's settings; every `None` stays as stored. The pull
/// interval is the focused one only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutosyncPatch {
    pub pull_enabled: Option<bool>,
    pub push_enabled: Option<bool>,
    pub pull_interval_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
    pub close_to_tray: Option<bool>,
}

/// Change the autosync settings `patch` names and leave the rest as stored.
pub async fn patch_autosync_settings(patch: AutosyncPatch) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        patch: AutosyncPatch,
    }
    engine::invoke("patch_autosync_settings", &Args { patch }).await
}

/// Flip one direction of autosync. `None` leaves a direction alone — the v2 main
/// page knows two booleans, and `update_autosync_settings` takes all five settings
/// fields, so writing through that one would mean fetching a payload this page has
/// no other use for.
pub async fn set_autosync_direction(pull: Option<bool>, push: Option<bool>) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        pull: Option<bool>,
        push: Option<bool>,
    }
    engine::invoke("set_autosync_direction", &Args { pull, push }).await
}

pub async fn update_fswatcher_settings(enabled: bool) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        enabled: bool,
    }
    engine::invoke("update_fswatcher_settings", &Args { enabled }).await
}

/// Turn off exactly the built-in ignore rules in `off`, for every package.
pub async fn set_default_ignore_rules(off: Vec<String>) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        off: Vec<String>,
    }
    engine::invoke("set_default_ignore_rules", &Args { off }).await
}

/// Open the user-wide ignore rules file, creating it if missing.
pub async fn open_user_ignore_rules() -> Result<(), String> {
    engine::invoke_unit("open_user_ignore_rules").await
}

/// Save the log level (`default`, `trace`, …); it applies after a restart.
pub async fn update_log_settings(level: String) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        level: String,
    }
    engine::invoke("update_log_settings", &Args { level }).await
}

/// Turn an experiment on or off. `None` leaves a flag as it is — a caller that
/// knows about one experiment must not reset another.
pub async fn update_experimental_settings(
    entire_package_sync: Option<bool>,
    agent_tasks: Option<bool>,
) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        entire_package_sync: Option<bool>,
        agent_tasks: Option<bool>,
    }
    engine::invoke(
        "update_experimental_settings",
        &Args {
            entire_package_sync,
            agent_tasks,
        },
    )
    .await
}

/// Replace `task_settings.json`.
pub async fn update_task_settings(settings: TaskSettings) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        settings: TaskSettings,
    }
    engine::invoke("update_task_settings", &Args { settings }).await
}

/// Choose the design generation. The next page that asks draws it.
pub async fn update_design_settings(design: Design) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        design: Design,
    }
    engine::invoke("update_design_settings", &Args { design }).await
}

/// Record which files a package keeps; `folders` is read only for a folders
/// scope. Storage only — catching up on files already listed is a separate
/// call.
pub async fn package_set_sync_scope(
    namespace: String,
    scope: KeepingScope,
    folders: Vec<String>,
) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        scope: KeepingScope,
        folders: Vec<String>,
    }
    engine::invoke(
        "package_set_sync_scope",
        &Args {
            namespace,
            scope,
            folders,
        },
    )
    .await
}

/// Payload of the `fswatcher-subscriber-error` engine event. Surfaced as a
/// one-shot toast (e.g. for `kind == "inotify_limit"` on Linux).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriberErrorEvent {
    pub kind: String,
    pub message: String,
    pub namespace: Option<Namespace>,
}

pub const FSWATCHER_SUBSCRIBER_ERROR_EVENT: &str = "fswatcher-subscriber-error";

pub async fn package_pull(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("package_pull", &Args { namespace, uri }).await
}

/// The dry-run verdict of what a Pull would do right now. UI-side mirror of the
/// engine's `quilt_rs::flow::PullOutcome` (the UI crate cannot depend on the
/// engine crate). The serde shape is the engine enum's default externally
/// tagged form with no field renaming; it MUST stay identical so the tagged
/// JSON crosses the engine boundary unchanged. Paths arrive as strings (the
/// engine's `PathBuf`s serialize as such); the UI only displays them.
///
/// The literals are anchored identically in the backend's
/// `pull_outcome_wire_form_is_verbatim`; if the two drift, the two-phase Pull
/// affordance silently misreads the outcome at the boundary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub enum PullOutcome {
    UpToDate,
    CleanUpdate,
    KeepsLocalChanges {
        added: Vec<String>,
        modified: Vec<String>,
        removed: Vec<String>,
    },
    Blocked {
        conflicts: Vec<String>,
    },
}

impl PullOutcome {
    /// Whether a Pull can proceed for this dry-run outcome. Only `Blocked` — a
    /// real two-sided conflict — disables the button; every other outcome,
    /// including `KeepsLocalChanges`, pulls safely.
    #[must_use]
    pub fn is_pullable(&self) -> bool {
        !matches!(self, PullOutcome::Blocked { .. })
    }
}

/// Tri-state view of the dry-run pull check for the Pull affordance. Replaces
/// the old `Option<PullOutcome>`, where `None` conflated "still loading" with
/// "the fetch failed" — a single failed dry-run then stranded the Pull button
/// on "Checking for updates…" forever, with no retry. `Loading` and `Failed`
/// both keep Pull disabled (fail-safe); `Failed` additionally offers a retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullCheck {
    /// The dry-run has not resolved yet (genuine in-flight state).
    Loading,
    /// The dry-run fetch errored; the button area offers a retry.
    Failed,
    /// The dry-run resolved: the verdict, and what the incoming revision adds.
    ///
    /// The whole preview rather than the verdict alone, because the surfaces
    /// name the incoming files from the same read that gates the Pull control —
    /// the paths are already in hand and a second fetch would buy nothing.
    Ready(PullPreview),
}

impl PullCheck {
    /// Whether Pull should be enabled: only a resolved, pullable outcome.
    #[must_use]
    pub fn pull_enabled(&self) -> bool {
        matches!(self, PullCheck::Ready(p) if p.outcome.is_pullable())
    }

    /// Whether the dry-run failed, so a retry affordance should show.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, PullCheck::Failed)
    }
}

/// The dry-run verdict, the paths the incoming revision adds, changes and
/// removes, and that revision's hash. Mirrors the engine's
/// `quilt_rs::flow::PullPreview`.
///
/// The paths are scope-independent — they name how the revision differs from
/// the one this copy holds, and whether a pull would fetch a file is the sync
/// scope's business at apply time. They cost nothing: the manifest naming them
/// was already fetched to reach the verdict. `latest_hash` is `None` when there
/// is no newer revision.
///
/// Each of `added`, `changed` and `removed` arrives sorted by path, compared
/// by component as `Path` compares (`a/b` before `a-b`), and a path is in at
/// most one of them: the engine reads them out of one ordered map.
/// The incoming popover merges them on that promise.
///
/// The literals are anchored identically in the backend's
/// `pull_preview_wire_form_is_verbatim`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullPreview {
    pub outcome: PullOutcome,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub latest_hash: Option<String>,
}

pub async fn package_pull_outcome(namespace: String) -> Result<PullPreview, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    engine::invoke("package_pull_outcome", &Args { namespace }).await
}

/// Remove a package; with `prune`, also delete its downloaded files that no
/// other package uses. What that freed arrives as a toast.
pub async fn package_uninstall(
    namespace: String,
    uri: Option<S3PackageUri>,
    prune: bool,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
        prune: bool,
    }
    engine::invoke(
        "package_uninstall",
        &Args {
            namespace,
            uri,
            prune,
        },
    )
    .await
}

pub async fn package_install_paths(uri: String, paths: Vec<String>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        uri: String,
        paths: Vec<String>,
    }
    engine::invoke("package_install_paths", &Args { uri, paths }).await
}

pub async fn package_create(
    namespace: String,
    source: Option<String>,
    message: Option<String>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        source: Option<String>,
        message: Option<String>,
    }
    engine::invoke(
        "package_create",
        &Args {
            namespace,
            source,
            message,
        },
    )
    .await
}

// ── Merge actions ───────────────────────────────────────────

pub async fn certify_latest(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("certify_latest", &Args { namespace, uri }).await
}

/// Step the package back one revision along its pending commit chain.
///
/// Refuses on a package with any remote, on a first revision, and on a dirty
/// tree — the first two are gated by the caller from the page payload, the
/// third comes back as the message the confirm dialog draws.
pub async fn undo_commit(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    engine::invoke("undo_commit", &Args { namespace }).await
}

pub async fn reset_local(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("reset_local", &Args { namespace, uri }).await
}

// ── Remote ──────────────────────────────────────────────────

/// Response from the `set_remote` command. UI-side mirror of the backend
/// `quilt_sync::commands::package_ops::SetRemoteResponse`; the serde attributes
/// MUST match so the typed payload crosses the engine boundary unchanged.
/// `resolution_warning` is `Some(reason)` when the remote was set but the
/// bucket's default workflow could not be resolved — the popup raises a warning
/// notice instead of a plain success.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRemoteResponse {
    pub message: String,
    pub resolution_warning: Option<String>,
}

pub async fn set_remote(
    namespace: String,
    origin: String,
    bucket: String,
    workflow: WorkflowIntent,
) -> Result<SetRemoteResponse, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        origin: String,
        bucket: String,
        workflow: WorkflowIntent,
    }
    engine::invoke(
        "set_remote",
        &Args {
            namespace,
            origin,
            bucket,
            workflow,
        },
    )
    .await
}

/// Add, change or remove the catalog of a package's remote; a blank `origin`
/// removes it. The bucket stays, so this works once the package is published.
pub async fn change_catalog(namespace: String, origin: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        origin: String,
    }
    engine::invoke("change_catalog", &Args { namespace, origin }).await
}

/// Fetch a bucket's declared workflows before its remote is set, so the
/// set-remote popup can present the same tri-state control the commit dialog
/// uses. Mirrors the backend `get_bucket_workflows` command; a fetch failure
/// maps to [`CommitWorkflows::Unavailable`] rather than an error.
/// A blank `host` is a bucket with no catalog.
pub async fn get_bucket_workflows(host: String, bucket: String) -> Result<CommitWorkflows, String> {
    #[derive(Serialize)]
    struct Args {
        host: Option<String>,
        bucket: String,
    }
    let host = Some(host).filter(|host| !host.trim().is_empty());
    engine::invoke("get_bucket_workflows", &Args { host, bucket }).await
}

// ── Auth ────────────────────────────────────────────────────

pub async fn login(host: String, code: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        code: String,
    }
    engine::invoke("login", &Args { host, code }).await
}

/// Start a browser login. It lands on `back` in this page, the one that asked.
pub async fn login_oauth(host: String, back: Option<String>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        back: Option<String>,
        page: Option<String>,
    }
    let page = Some(engine::page_id());
    engine::invoke("login_oauth", &Args { host, back, page }).await
}

/// Somewhere the engine sent this page.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Navigation {
    pub id: String,
    /// The address, without the host.
    pub page: String,
}

/// Where the engine sent this page, if anywhere, until it says it went.
pub async fn pending_navigation() -> Result<Option<Navigation>, String> {
    #[derive(Serialize)]
    struct Args {
        page: String,
    }
    let page = engine::page_id();
    engine::invoke("pending_navigation", &Args { page }).await
}

/// This page followed navigation `id`.
pub async fn navigation_done(id: String) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        page: String,
        id: String,
    }
    let page = engine::page_id();
    engine::invoke("navigation_done", &Args { page, id }).await
}

pub async fn erase_auth(host: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
    }
    engine::invoke("erase_auth", &Args { host }).await
}

/// The roles a host's stack grants the logged-in user. UI-side mirror of the
/// backend `quilt_sync::commands::auth::RolesData`; the serde attributes MUST
/// match so the payload crosses the engine boundary unchanged. `available`
/// includes `current`, so a single-element list means there is nothing to
/// switch to.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RolesData {
    pub current: String,
    pub available: Vec<String>,
}

/// Fetch the host's roles. Takes that host's credential single-flight lock, so
/// call it lazily — never as part of a bulk page load.
pub async fn get_roles(host: String) -> Result<RolesData, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
    }
    engine::invoke("get_roles", &Args { host }).await
}

/// Switch the host's primary role. The backend expires the stored credentials
/// and clears the host's cached S3 clients, so the UI has no cache work to do.
/// Returns the roles as confirmed by the stack — the confirmed `current` can
/// differ from the requested role, so render what comes back, not what was
/// asked for.
pub async fn switch_role(host: String, role: String) -> Result<RolesData, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        role: String,
    }
    engine::invoke("switch_role", &Args { host, role }).await
}

// ── Setup ───────────────────────────────────────────────────

pub async fn setup(directory: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        directory: String,
    }
    engine::invoke("setup", &Args { directory }).await
}

// ── Quiltignore ─────────────────────────────────────────────

pub async fn add_to_quiltignore(
    namespace: String,
    pattern: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        pattern: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "add_to_quiltignore",
        &Args {
            namespace,
            pattern,
            uri,
        },
    )
    .await
}

/// What *Stop ignoring* writes in one scope, and what the next sync adds.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopePreview {
    /// The `!` lines, outermost first.
    pub lines: Vec<String>,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopIgnoringPreview {
    pub this_package: ScopePreview,
    /// `None` when no user-wide line can bring the path back.
    pub all_packages: Option<ScopePreview>,
}

/// Where *Stop ignoring* writes: the package's `.quiltignore`, or the
/// user-wide one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopIgnoringScope {
    ThisPackage,
    AllPackages,
}

pub async fn stop_ignoring_preview(
    namespace: String,
    path: String,
    folder: bool,
) -> Result<StopIgnoringPreview, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        folder: bool,
    }
    engine::invoke(
        "stop_ignoring_preview",
        &Args {
            namespace,
            path,
            folder,
        },
    )
    .await
}

pub async fn stop_ignoring(
    namespace: String,
    path: String,
    folder: bool,
    scope: StopIgnoringScope,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        folder: bool,
        scope: StopIgnoringScope,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "stop_ignoring",
        &Args {
            namespace,
            path,
            folder,
            scope,
            uri,
        },
    )
    .await
}

pub async fn test_quiltignore_pattern(pattern: String, path: String) -> Result<bool, String> {
    #[derive(Serialize)]
    struct Args {
        pattern: String,
        path: String,
    }
    engine::invoke("test_quiltignore_pattern", &Args { pattern, path }).await
}

// ── File/browser ────────────────────────────────────────────

pub async fn open_in_file_browser(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke("open_in_file_browser", &Args { namespace, uri }).await
}

pub async fn open_in_web_browser(url: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        url: String,
    }
    engine::invoke("open_in_web_browser", &Args { url }).await
}

pub async fn open_in_default_application(
    namespace: String,
    path: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "open_in_default_application",
        &Args {
            namespace,
            path,
            uri,
        },
    )
    .await
}

pub async fn reveal_in_file_browser(
    namespace: String,
    path: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        uri: Option<S3PackageUri>,
    }
    engine::invoke(
        "reveal_in_file_browser",
        &Args {
            namespace,
            path,
            uri,
        },
    )
    .await
}

pub async fn open_directory_picker() -> Result<String, String> {
    engine::invoke_unit("open_directory_picker").await
}

// ── Debug/diagnostics ───────────────────────────────────────

pub async fn debug_dot_quilt() -> Result<String, String> {
    engine::invoke_unit("debug_dot_quilt").await
}

pub async fn debug_logs() -> Result<String, String> {
    engine::invoke_unit("debug_logs").await
}

pub async fn open_home_dir() -> Result<String, String> {
    engine::invoke_unit("open_home_dir").await
}

pub async fn open_data_dir() -> Result<String, String> {
    engine::invoke_unit("open_data_dir").await
}

/// What Free up space freed: the sentence, and its bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreedSpace {
    pub message: String,
    pub freed_bytes: u64,
}

/// Delete what local storage holds for nothing, and say what was freed.
pub async fn run_gc() -> Result<FreedSpace, String> {
    engine::invoke_unit("run_gc").await
}

/// How much local storage holds, and how much Free up space would free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSize {
    pub total_bytes: u64,
    /// An estimate: Free up space's own report is the exact answer.
    pub freeable_bytes: u64,
}

/// Measure local storage, deleting nothing; run only when the user asks.
pub async fn measure_storage() -> Result<StorageSize, String> {
    engine::invoke_unit("measure_storage").await
}

pub async fn collect_diagnostic_logs() -> Result<String, String> {
    engine::invoke_unit("collect_diagnostic_logs").await
}

/// Send the diagnostic zip to Quilt; `message` is what the user wrote about the
/// problem, if anything.
pub async fn send_crash_report(
    zip_path: String,
    message: Option<String>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        zip_path: String,
        message: Option<String>,
    }
    engine::invoke("send_crash_report", &Args { zip_path, message }).await
}

#[cfg(test)]
mod tests {
    use super::{
        CommitViolation, CommitWorkflows, EntryCounts, EntryList, FilesData, KeepingScope,
        KeptReason, PackageContextData, PackageItemData, PackageSize, PullOutcome, PullPreview,
        ResolveData, RevisionHistoryData, RolesData, ViolationField, WorkflowInfo, WorkflowIntent,
    };
    use crate::kit::RowDownloads;

    /// Anchored identically in the backend's `files_data_wire_form_is_verbatim`.
    #[test]
    fn files_data_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<FilesData>(r#"{"kind":"unlisted","reason":"denied"}"#).unwrap(),
            FilesData::Unlisted {
                reason: "denied".to_string()
            }
        );
        assert_eq!(
            serde_json::from_str::<FilesData>(
                r#"{"kind":"listed","entries":[],"counts":{"all":0,"changed":0,"notDownloaded":0,"ignored":0,"ignoredFolders":0,"deleted":0},"total":0,"truncated":false}"#
            )
            .unwrap(),
            FilesData::Listed(EntryList {
                entries: Vec::new(),
                counts: EntryCounts::default(),
                total: 0,
                truncated: false,
            })
        );
    }

    /// Anchored identically in the backend's `entry_counts_wire_form_is_verbatim`.
    #[test]
    fn entry_counts_wire_form_is_verbatim() {
        let counts = serde_json::from_str::<EntryCounts>(
            r#"{"all":1410,"changed":700,"notDownloaded":700,"ignored":10,"ignoredFolders":2,"deleted":200}"#,
        )
        .unwrap();

        assert_eq!(
            counts,
            EntryCounts {
                all: 1410,
                changed: 700,
                not_downloaded: 700,
                ignored: 10,
                ignored_folders: 2,
                deleted: 200,
            }
        );
    }

    /// Anchored identically in the backend's
    /// `current_revision_context_wire_form_is_verbatim` test, `keeping` included.
    #[test]
    fn current_revision_context_wire_form_is_verbatim() {
        let context = serde_json::from_str::<PackageContextData>(
            r#"{"revision":{"hash":"abc123","message":"Initial upload","obtainedAt":1758500000000.0},"bucket":"quilt-lab-plates","revisionCount":4,"keeping":{"scope":"entirePackage","folders":[],"total":56,"outstanding":2,"remoteOnly":["plate/b.csv","plate/c.csv"],"deletedHere":1},"size":{"total":3400000,"downloaded":1900000,"backlog":1500000},"resolve":null}"#,
        )
        .unwrap();

        assert_eq!(context.revision.hash, "abc123");
        assert_eq!(context.revision.message.as_deref(), Some("Initial upload"));
        assert!(
            (context.revision.obtained_at - 1_758_500_000_000.0).abs() < f64::EPSILON,
            "the timestamp crosses unchanged"
        );
        assert_eq!(context.bucket.as_deref(), Some("quilt-lab-plates"));
        assert_eq!(context.revision_count, 4);
        assert_eq!(context.keeping.scope, KeepingScope::EntirePackage);
        assert_eq!(context.keeping.total, 56);
        assert_eq!(context.keeping.outstanding, 2);
        assert_eq!(context.keeping.remote_only, ["plate/b.csv", "plate/c.csv"]);
        assert_eq!(context.keeping.deleted_here, 1);
        assert_eq!(
            context.size,
            Some(PackageSize {
                total: 3_400_000,
                downloaded: 1_900_000,
                backlog: 1_500_000,
            })
        );
        assert_eq!(context.resolve, None);
    }

    /// A size the backend could not read crosses as `null`, and the caption
    /// then gives the count alone.
    #[test]
    fn an_unread_package_size_crosses_as_null() {
        let context = serde_json::from_str::<PackageContextData>(
            r#"{"revision":{"hash":"abc123","message":null,"obtainedAt":0.0},"bucket":null,"revisionCount":1,"keeping":{"scope":"individualFiles","folders":[],"total":0,"outstanding":0,"remoteOnly":[],"deletedHere":0},"size":null,"resolve":null}"#,
        )
        .unwrap();

        assert_eq!(context.size, None);
    }

    /// Anchored identically in the backend's
    /// `resolve_compared_wire_form_is_verbatim` test.
    #[test]
    fn resolve_compared_wire_form_is_verbatim() {
        let resolve = serde_json::from_str::<ResolveData>(
            r#"{"kind":"compared","publishedMessage":"Add intake folder-upload note","differing":["plate/a.csv","plate/b.csv"],"unpublished":2,"uncommitted":1}"#,
        )
        .unwrap();

        assert_eq!(
            resolve,
            ResolveData::Compared {
                published_message: Some("Add intake folder-upload note".to_string()),
                differing: vec!["plate/a.csv".to_string(), "plate/b.csv".to_string()],
                unpublished: 2,
                uncommitted: 1,
            }
        );
    }

    /// Anchored identically in the backend's
    /// `resolve_refused_wire_form_is_verbatim` test.
    #[test]
    fn resolve_refused_wire_form_is_verbatim() {
        let resolve =
            serde_json::from_str::<ResolveData>(r#"{"kind":"refused","reason":"AccessDenied"}"#)
                .unwrap();

        assert_eq!(
            resolve,
            ResolveData::Refused {
                reason: "AccessDenied".to_string(),
            }
        );
    }

    /// Anchored identically in the backend's
    /// `revision_history_wire_form_is_verbatim` test.
    #[test]
    fn revision_history_wire_form_is_verbatim() {
        let data = serde_json::from_str::<RevisionHistoryData>(
            r#"{"rows":[{"hash":"published-hash","message":"Sent","obtainedAt":1758500000000.0,"published":true,"catalogUrl":"https://quilt.test/b/test/packages/team/dataset/tree/published-hash","kept":[],"frees":1200000},{"hash":"local-hash","message":null,"obtainedAt":1758400000000.0,"published":false,"catalogUrl":null,"kept":["current","notPushed"],"frees":null}],"removableFrees":1200000}"#,
        )
        .unwrap();

        let [published, unpublished] = data.rows.as_slice() else {
            panic!("two rows, got {data:?}");
        };
        assert_eq!(published.hash, "published-hash");
        assert_eq!(published.message.as_deref(), Some("Sent"));
        assert!(
            (published.obtained_at - 1_758_500_000_000.0).abs() < f64::EPSILON,
            "the timestamp crosses unchanged"
        );
        assert!(published.published);
        assert_eq!(
            published.catalog_url.as_deref(),
            Some("https://quilt.test/b/test/packages/team/dataset/tree/published-hash")
        );
        assert_eq!(published.kept, Vec::new());
        assert_eq!(published.frees, Some(1_200_000));
        assert_eq!(unpublished.message, None);
        assert!(
            (unpublished.obtained_at - 1_758_400_000_000.0).abs() < f64::EPSILON,
            "the timestamp crosses unchanged"
        );
        assert!(!unpublished.published);
        assert_eq!(unpublished.catalog_url, None);
        assert_eq!(
            unpublished.kept,
            vec![KeptReason::Current, KeptReason::NotPushed]
        );
        assert_eq!(unpublished.frees, None);
        assert_eq!(data.removable_frees, Some(1_200_000));
    }

    /// The mirror struct must deserialize the exact JSON the backend
    /// (`quilt_sync::commands::package_list::InstalledPackageListItem`)
    /// serializes. This literal is anchored identically in the backend's
    /// `package_item_data_wire_form_is_verbatim`; if the two drift, the
    /// list silently drops the pause reason (or a whole field) at the
    /// engine boundary — the exact class of bug this data-driven design
    /// exists to prevent.
    #[test]
    fn package_item_data_wire_form_is_verbatim() {
        let item = serde_json::from_str::<PackageItemData>(
            r#"{"namespace":"acme/data","status":"paused","hasChanges":false,"hasLocalCommit":false,"uri":null,"remoteDisplay":null,"pausedReason":"workflow rejected metadata","pausedKind":"other","noAccess":true,"noAccessReason":"Current role ReadOnly has no access to this bucket","roleSwitchHost":"quilt.test"}"#,
        )
        .unwrap();
        assert_eq!(item.namespace.to_string(), "acme/data");
        assert_eq!(item.status, "paused");
        assert!(!item.has_changes);
        assert!(!item.has_local_commit);
        assert!(item.uri.is_none());
        assert!(item.remote_display.is_none());
        assert_eq!(
            item.paused_reason.as_deref(),
            Some("workflow rejected metadata")
        );
        assert_eq!(item.paused_kind.as_deref(), Some("other"));
        assert!(item.no_access);
        assert_eq!(
            item.no_access_reason.as_deref(),
            Some("Current role ReadOnly has no access to this bucket")
        );
        assert_eq!(item.role_switch_host.as_deref(), Some("quilt.test"));
    }

    /// The mirror types must deserialize the exact tagged JSON the backend
    /// (`quilt_sync::commands::commit_data::CommitViolation`) serializes. These
    /// literals are anchored identically in the backend's
    /// `commit_violation_wire_form_is_verbatim`; if they drift, the dialog routes
    /// live violations to the wrong field or drops them.
    #[test]
    fn commit_violation_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<CommitViolation>(r#"{"field":"metadata","message":"bad"}"#)
                .unwrap(),
            CommitViolation {
                field: ViolationField::Metadata,
                message: "bad".to_string(),
            }
        );
        assert_eq!(
            serde_json::from_str::<ViolationField>(r#""message""#).unwrap(),
            ViolationField::Message
        );
        assert_eq!(
            serde_json::from_str::<ViolationField>(r#""name""#).unwrap(),
            ViolationField::Name
        );
    }

    /// The mirror enum must deserialize the exact tagged JSON the backend
    /// (`quilt_sync::commands::commit_data::CommitWorkflows`) serializes. These
    /// literals are anchored identically in the backend's
    /// `commit_workflows_wire_form_is_verbatim`; if the two drift, the commit
    /// dialog silently loses the workflow list.
    #[test]
    fn commit_workflows_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(
                r#"{"state":"available","workflows":[{"id":"alpha","name":"Alpha","description":null,"metadataSchemaUrl":"https://catalog/b/bucket/tree/meta.json","entriesSchemaUrl":null}],"defaultWorkflow":"alpha","isWorkflowRequired":true,"configUrl":"https://catalog/b/bucket/tree/.quilt/workflows/config.yml"}"#
            )
            .unwrap(),
            CommitWorkflows::Available {
                workflows: vec![WorkflowInfo {
                    id: "alpha".to_string(),
                    name: Some("Alpha".to_string()),
                    description: None,
                    metadata_schema_url: Some(
                        "https://catalog/b/bucket/tree/meta.json".to_string()
                    ),
                    entries_schema_url: None,
                }],
                default_workflow: Some("alpha".to_string()),
                is_workflow_required: true,
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string()
                ),
            }
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(r#"{"state":"notConfigured"}"#).unwrap(),
            CommitWorkflows::NotConfigured
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(r#"{"state":"unavailable"}"#).unwrap(),
            CommitWorkflows::Unavailable
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(
                r#"{"state":"invalid","reason":"bad schema","configUrl":"https://catalog/b/bucket/tree/.quilt/workflows/config.yml"}"#
            )
            .unwrap(),
            CommitWorkflows::Invalid {
                reason: "bad schema".to_string(),
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string()
                ),
            }
        );
    }

    /// The mirror enum must deserialize the exact externally-tagged JSON the
    /// backend (`quilt_rs::flow::PullOutcome`) serializes. These literals are
    /// anchored identically in the backend's `pull_outcome_wire_form_is_verbatim`;
    /// if the two drift, the two-phase Pull affordance silently misreads the
    /// dry-run outcome at the engine boundary.
    #[test]
    fn pull_outcome_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#""UpToDate""#).unwrap(),
            PullOutcome::UpToDate
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#""CleanUpdate""#).unwrap(),
            PullOutcome::CleanUpdate
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(
                r#"{"KeepsLocalChanges":{"added":["a.txt"],"modified":[],"removed":["c.txt"]}}"#
            )
            .unwrap(),
            PullOutcome::KeepsLocalChanges {
                added: vec!["a.txt".to_string()],
                modified: vec![],
                removed: vec!["c.txt".to_string()],
            }
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#"{"Blocked":{"conflicts":["x.txt"]}}"#).unwrap(),
            PullOutcome::Blocked {
                conflicts: vec!["x.txt".to_string()],
            }
        );
    }

    /// Anchored identically in the backend's `pull_preview_wire_form_is_verbatim`:
    /// the page names the incoming files and links them from this shape.
    #[test]
    fn pull_preview_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<PullPreview>(
                r#"{"outcome":"CleanUpdate","added":["a.txt"],"changed":["b.txt"],"removed":["c.txt"],"latestHash":"abc123"}"#
            )
            .unwrap(),
            PullPreview {
                outcome: PullOutcome::CleanUpdate,
                added: vec!["a.txt".to_string()],
                changed: vec!["b.txt".to_string()],
                removed: vec!["c.txt".to_string()],
                latest_hash: Some("abc123".to_string()),
            }
        );
        assert_eq!(
            serde_json::from_str::<PullPreview>(
                r#"{"outcome":"UpToDate","added":[],"changed":[],"removed":[],"latestHash":null}"#
            )
            .unwrap(),
            PullPreview {
                outcome: PullOutcome::UpToDate,
                added: Vec::new(),
                changed: Vec::new(),
                removed: Vec::new(),
                latest_hash: None,
            }
        );
    }

    #[test]
    fn pull_outcome_is_pullable_only_blocks_on_conflicts() {
        assert!(PullOutcome::UpToDate.is_pullable());
        assert!(PullOutcome::CleanUpdate.is_pullable());
        assert!(
            PullOutcome::KeepsLocalChanges {
                added: vec!["a.txt".to_string()],
                modified: vec![],
                removed: vec![],
            }
            .is_pullable()
        );
        assert!(
            !PullOutcome::Blocked {
                conflicts: vec!["x.txt".to_string()],
            }
            .is_pullable()
        );
    }

    /// The mirror struct must deserialize the exact JSON the backend
    /// (`quilt_sync::commands::auth::RolesData`) serializes. If the two drift,
    /// the Settings role switcher silently sees one role and hides itself.
    #[test]
    fn roles_data_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<RolesData>(
                r#"{"current":"ReadWrite","available":["ReadOnly","ReadWrite"]}"#
            )
            .unwrap(),
            RolesData {
                current: "ReadWrite".to_string(),
                available: vec!["ReadOnly".to_string(), "ReadWrite".to_string()],
            }
        );
    }

    /// The mirror struct must deserialize the exact JSON the backend's light-phase
    /// wire-shape test
    /// (`quilt_sync::commands::main_page::get_main_page_packages_from_model_serializes_the_wire_shape`)
    /// pins for one row. If the two drift, a light-phase row silently fails to
    /// deserialize at the engine boundary.
    #[test]
    fn main_page_packages_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPagePackagesData>(
            r#"{"packages":[{"namespace":"team/latest","state":{"kind":"latest"},"changedAt":null,"bucket":"test","host":"quilt.test","provisional":true,"roleSwitchHost":null}]}"#,
        )
        .unwrap();
        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace.to_string(), "team/latest");
        assert_eq!(pkg.state, crate::kit::PackageState::Latest);
        assert_eq!(pkg.changed_at, None);
        assert_eq!(pkg.host.as_deref(), Some("quilt.test"));
        assert!(pkg.provisional);
        assert_eq!(pkg.role_switch_host, None);
    }

    /// The mirror struct must deserialize the exact camelCase JSON the
    /// backend's `MainPageFile`/`MainPageRecentFiles`
    /// (`#[serde(rename_all = "camelCase")]`, both plain `Serialize`) produce.
    /// If the two drift, the feed silently fails to deserialize at the engine
    /// boundary.
    #[test]
    fn main_page_recent_files_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPageRecentFilesData>(
            r#"{"files":[{"path":"a/one.csv","namespace":"user/alpha","changedAt":1000.0,"publishedHash":"abc123"}]}"#,
        )
        .unwrap();
        assert_eq!(data.files.len(), 1);
        let file = &data.files[0];
        assert_eq!(file.path, "a/one.csv");
        assert_eq!(file.namespace.to_string(), "user/alpha");
        assert!((file.changed_at - 1000.0).abs() < f64::EPSILON);
        assert_eq!(file.published_hash.as_deref(), Some("abc123"));
    }

    /// The mirror struct must deserialize the heavy phase's two never-before-seen
    /// kinds. UI-side mirror of the backend
    /// `quilt_sync::commands::main_page::MainPagePackageRefresh`; these two
    /// literals never crossed the wire before the heavy phase existed, because the
    /// light phase could not produce either. Deserializing to `PackageState::Unknown`
    /// is `#[serde(other)]`'s silent-drift failure mode — that is what this pins
    /// against, not just "it parses".
    #[test]
    fn main_page_package_refresh_data_wire_form_is_verbatim() {
        let pending = serde_json::from_str::<super::MainPagePackageRefreshData>(
            r#"{"state":{"kind":"pending_changes","files":3},"roleSwitchHost":null}"#,
        )
        .unwrap();
        assert!(
            !matches!(pending.state, crate::kit::PackageState::Unknown),
            "a kind drift would silently land here, via #[serde(other)]"
        );
        assert_eq!(
            pending.state,
            crate::kit::PackageState::PendingChanges { files: 3 }
        );

        let denied = serde_json::from_str::<super::MainPagePackageRefreshData>(
            r#"{"state":{"kind":"role_denied","role":null},"roleSwitchHost":"h"}"#,
        )
        .unwrap();
        assert!(
            !matches!(denied.state, crate::kit::PackageState::Unknown),
            "a kind drift would silently land here, via #[serde(other)]"
        );
        assert_eq!(
            denied.state,
            crate::kit::PackageState::RoleDenied { role: None }
        );
        assert_eq!(denied.role_switch_host.as_deref(), Some("h"));
        assert_eq!(denied.downloads, None);

        let counted = serde_json::from_str::<super::MainPagePackageRefreshData>(
            r#"{"state":{"kind":"latest"},"roleSwitchHost":null,"downloads":{"scope":"individualFiles","folders":0,"total":340,"outstanding":328,"backlog":0,"deletedHere":1,"size":{"total":3400000,"downloaded":1900000,"backlog":0}}}"#,
        )
        .unwrap();
        assert_eq!(
            counted.downloads,
            Some(super::MainPageDownloadsData {
                scope: KeepingScope::IndividualFiles,
                folders: 0,
                total: 340,
                outstanding: 328,
                backlog: 0,
                deleted_here: 1,
                size: Some(super::PackageSize {
                    total: 3_400_000,
                    downloaded: 1_900_000,
                    backlog: 0,
                }),
            })
        );

        let unread = serde_json::from_str::<super::MainPageDownloadsData>(
            r#"{"scope":"entirePackage","folders":0,"total":2,"outstanding":0,"backlog":0,"deletedHere":0,"size":null}"#,
        )
        .unwrap();
        assert_eq!(unread.size, None);
    }

    /// A row whose outstanding files are all in the backlog, unless the scope
    /// is picked files.
    fn row_downloads(scope: KeepingScope, total: usize, outstanding: usize) -> RowDownloads {
        super::MainPageDownloadsData {
            scope,
            folders: usize::from(scope == KeepingScope::Folders),
            total,
            outstanding,
            backlog: if scope == KeepingScope::IndividualFiles {
                0
            } else {
                outstanding
            },
            deleted_here: 0,
            size: None,
        }
        .into()
    }

    #[test]
    fn a_whole_package_marks_what_it_has_still_to_download() {
        assert_eq!(
            row_downloads(KeepingScope::EntirePackage, 5, 0),
            RowDownloads::EntirePackage { to_download: 0 }
        );
        assert_eq!(
            row_downloads(KeepingScope::EntirePackage, 5, 2),
            RowDownloads::EntirePackage { to_download: 2 }
        );
    }

    #[test]
    fn kept_folders_mark_what_they_have_still_to_download() {
        assert_eq!(
            row_downloads(KeepingScope::Folders, 5, 2),
            RowDownloads::Folders {
                folders: 1,
                to_download: 2
            }
        );
    }

    #[test]
    fn picked_files_mark_what_is_here_out_of_the_revision() {
        assert_eq!(
            row_downloads(KeepingScope::IndividualFiles, 340, 328),
            RowDownloads::IndividualFiles {
                downloaded: 12,
                total: 340
            }
        );
        assert_eq!(
            row_downloads(KeepingScope::IndividualFiles, 340, 0),
            RowDownloads::IndividualFiles {
                downloaded: 340,
                total: 340
            }
        );
    }

    /// The mirror enum must serialize to the exact tagged JSON the backend
    /// (`quilt_rs::io::remote::WorkflowIntent`) deserializes, and round-trip
    /// back. If these strings drift, the engine boundary for commits breaks silently.
    #[test]
    fn workflow_intent_wire_form_is_verbatim() {
        let cases = [
            (
                WorkflowIntent::BucketDefault,
                r#"{"kind":"bucket-default"}"#,
            ),
            (WorkflowIntent::NoWorkflow, r#"{"kind":"no-workflow"}"#),
            (
                WorkflowIntent::Named("x".to_string()),
                r#"{"kind":"named","id":"x"}"#,
            ),
        ];
        for (intent, json) in cases {
            assert_eq!(serde_json::to_string(&intent).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<WorkflowIntent>(json).unwrap(),
                intent
            );
        }
    }

    /// The mirror struct must deserialize the exact JSON the backend's
    /// `quilt_sync::commands::main_page::the_watcher_payload_serializes_the_wire_shape`
    /// pins.
    #[test]
    fn main_page_watcher_data_wire_form_is_verbatim() {
        use super::AutosyncStateData;
        let data = serde_json::from_str::<super::MainPageWatcherData>(
            r#"{"pull":{"enabled":true,"activity":"armed","deadline":1754500030000.0,"intervalMs":30000.0},"publish":{"enabled":true,"intervalMs":300000.0,"packages":[{"namespace":"team/a","state":{"kind":"waiting","publishAt":1754500030000.0,"changedWhileClosed":true}},{"namespace":"team/b","state":{"kind":"publishing"}},{"namespace":"team/c","state":{"kind":"retrying","error":"Could not reach S3","nextTryAt":1754500030000.0}},{"namespace":"team/plate-07","state":{"kind":"paused","reason":{"kind":"pull_conflict","files":["a.csv","b.csv"]}}},{"namespace":"team/e","state":{"kind":"off"}}]},"paused":[{"namespace":"team/plate-07","reason":{"kind":"pull_conflict","files":["a.csv","b.csv"]}}]}"#,
        )
        .unwrap();
        assert!(data.pull.enabled);
        assert_eq!(data.pull.activity, super::ToggleActivityData::Armed);
        assert_eq!(data.pull.deadline, Some(1_754_500_030_000.0));
        assert!((data.pull.interval_ms - 30_000.0).abs() < f64::EPSILON);
        assert!((data.publish.interval_ms - 300_000.0).abs() < f64::EPSILON);
        let states: Vec<_> = data
            .publish
            .packages
            .iter()
            .map(|p| (p.namespace.to_string(), p.state.clone()))
            .collect();
        let conflict = super::PausedReasonData::PullConflict {
            files: vec!["a.csv".to_string(), "b.csv".to_string()],
        };
        assert_eq!(
            states,
            [
                (
                    "team/a".to_string(),
                    AutosyncStateData::Waiting {
                        publish_at: 1_754_500_030_000.0,
                        changed_while_closed: true,
                    }
                ),
                ("team/b".to_string(), AutosyncStateData::Publishing),
                (
                    "team/c".to_string(),
                    AutosyncStateData::Retrying {
                        error: "Could not reach S3".to_string(),
                        next_try_at: 1_754_500_030_000.0,
                    }
                ),
                (
                    "team/plate-07".to_string(),
                    AutosyncStateData::Paused {
                        reason: conflict.clone()
                    }
                ),
                ("team/e".to_string(), AutosyncStateData::Off),
            ]
        );
        assert_eq!(data.paused.len(), 1);
        assert_eq!(data.paused[0].reason, conflict);
    }

    #[test]
    fn an_unknown_package_state_degrades_instead_of_failing_the_payload() {
        let state =
            serde_json::from_str::<super::AutosyncStateData>(r#"{"kind":"thinking"}"#).unwrap();
        assert_eq!(state, super::AutosyncStateData::Unrecognised);
    }

    #[test]
    fn a_conflict_arrives_as_a_list_with_its_commas_intact() {
        // The conflict's files arrive as a list, so a file name containing ", "
        // stays one path.
        let data = serde_json::from_str::<super::PausedReasonData>(
            r#"{"kind":"pull_conflict","files":["plate, run 3.csv","b.csv"]}"#,
        )
        .unwrap();
        match data {
            super::PausedReasonData::PullConflict { files } => {
                assert_eq!(files.len(), 2);
                assert_eq!(files[0], "plate, run 3.csv");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn a_reason_this_build_has_never_heard_of_degrades_instead_of_failing_the_payload() {
        // A reason added to the backend without an arm here would otherwise fail the
        // WHOLE payload and the card would vanish. Same treatment
        // `PackageState::Unknown` gets, for the same reason.
        assert_eq!(
            serde_json::from_str::<super::PausedReasonData>(r#"{"kind":"some_future_reason"}"#)
                .unwrap(),
            super::PausedReasonData::Unrecognised
        );
        // And the guard against `#[serde(other)]`'s silent-drift failure mode: a
        // kind we DO know must not land in the catch-all.
        assert_ne!(
            serde_json::from_str::<super::PausedReasonData>(r#"{"kind":"diverged"}"#).unwrap(),
            super::PausedReasonData::Unrecognised
        );
    }

    #[test]
    fn the_three_named_fields_arrive_separately() {
        for (json, expected) in [
            (
                r#"{"kind":"other","message":"workflow rejected metadata"}"#,
                super::PausedReasonData::Other {
                    message: "workflow rejected metadata".to_string(),
                },
            ),
            (
                r#"{"kind":"role_denied","role":"analyst"}"#,
                super::PausedReasonData::RoleDenied {
                    role: Some("analyst".to_string()),
                },
            ),
            (
                r#"{"kind":"role_denied","role":null}"#,
                super::PausedReasonData::RoleDenied { role: None },
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<super::PausedReasonData>(json).unwrap(),
                expected
            );
        }
    }

    /// The mirror must deserialize the exact JSON the backend's
    /// `the_accounts_payload_serializes_the_wire_shape` pins. Character-for-character:
    /// a literal the backend does not emit looks like a guard and proves nothing.
    #[test]
    fn main_page_accounts_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPageAccountsData>(
            r#"{"hosts":[{"host":"one.quilt.test","signedIn":true,"currentRole":null,"roles":[],"provisional":true},{"host":"another.quilt.test","signedIn":false,"currentRole":null,"roles":[],"provisional":false}]}"#,
        )
        .unwrap();
        assert_eq!(data.hosts.len(), 2);
        assert!(data.hosts[0].signed_in);
        assert!(
            data.hosts[0].provisional,
            "a signed-in host waits for its role"
        );
        assert_eq!(data.hosts[0].current_role, None);
        assert!(!data.hosts[1].signed_in);
        assert!(
            !data.hosts[1].provisional,
            "a signed-out host is already final"
        );
    }

    #[test]
    fn a_settled_account_arrives_with_its_role_and_alternatives() {
        let host = serde_json::from_str::<super::AccountHostData>(
            r#"{"host":"quilt.test","signedIn":true,"currentRole":"analyst","roles":["analyst","admin"],"provisional":false}"#,
        )
        .unwrap();
        assert_eq!(host.current_role.as_deref(), Some("analyst"));
        assert_eq!(host.roles, vec!["analyst".to_string(), "admin".to_string()]);
        assert!(!host.provisional);
    }

    #[test]
    fn a_nameless_role_is_null_not_an_empty_string() {
        // R5's wire form. `HostRow` maps this to "Role unavailable"; an empty string
        // would be indistinguishable from a role literally named "".
        let host = serde_json::from_str::<super::AccountHostData>(
            r#"{"host":"quilt.test","signedIn":true,"currentRole":null,"roles":[],"provisional":false}"#,
        )
        .unwrap();
        assert!(host.signed_in);
        assert_eq!(host.current_role, None);
        assert!(!host.provisional);
    }
}

// ── Toasts (server-emitted notifications) ──

/// Event name for a newly posted toast. Kept in lockstep with the backend's
/// `toast::TOAST_EVENT`.
pub const TOAST_EVENT: &str = "toast";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// A list under a heading. Mirrors the backend's `toast::ToastGroup`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToastGroup {
    pub heading: String,
    pub items: Vec<String>,
    /// How many items the heading counts but `items` does not show.
    pub more: usize,
}

/// One server-emitted notification. Mirrors the backend's `toast::Toast`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub title: Option<String>,
    /// The lead sentence: what happened.
    pub body: String,
    /// The detail, as lists rather than as indented text in `body`.
    #[serde(default)]
    pub groups: Vec<ToastGroup>,
    /// `None` stands until dismissed.
    pub timeout_ms: Option<u32>,
}

/// Every undismissed toast, oldest first — the mount-time hydration read that
/// catches whatever was posted while no window was open.
pub async fn get_toasts() -> Result<Vec<Toast>, String> {
    #[derive(Serialize)]
    struct Args {}
    engine::invoke("get_toasts", &Args {}).await
}

pub async fn dismiss_toast(id: u64) -> Result<(), String> {
    #[derive(Serialize)]
    struct Args {
        id: u64,
    }
    engine::invoke("dismiss_toast", &Args { id }).await
}

// ── Autopull activity (what autopull is doing right now) ──

/// Event carrying autopull's current activity. Kept in lockstep with the backend.
pub const AUTOPULL_ACTIVITY_EVENT: &str = "autopull-activity";

/// What autopull is doing to a package.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityOp {
    Pull,
    Publish,
}

/// The one pull or publish autopull is running. Mirrors the backend's `AutopullActivity`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutopullActivity {
    pub op: ActivityOp,
    pub namespace: Namespace,
}

/// Autopull's activity now, or `None` when it is idle — the mount-time hydration read.
pub async fn get_autopull_activity() -> Result<Option<AutopullActivity>, String> {
    #[derive(Serialize)]
    struct Args {}
    engine::invoke("get_autopull_activity", &Args {}).await
}

#[cfg(test)]
mod activity_tests {
    use super::{ActivityOp, AutopullActivity};

    /// Pins the backend's wire form: camelCase, the op as a lowercase string.
    #[test]
    fn an_activity_reads_from_the_wire() {
        let pull =
            serde_json::from_str::<AutopullActivity>(r#"{"op":"pull","namespace":"team/pkg"}"#)
                .unwrap();
        assert_eq!(pull.op, ActivityOp::Pull);
        assert_eq!(pull.namespace.to_string(), "team/pkg");

        let publish =
            serde_json::from_str::<AutopullActivity>(r#"{"op":"publish","namespace":"team/pkg"}"#)
                .unwrap();
        assert_eq!(publish.op, ActivityOp::Publish);
        assert_eq!(publish.namespace.to_string(), "team/pkg");
    }

    /// The backend sends `null` when autopull is idle.
    #[test]
    fn null_reads_as_nothing() {
        assert_eq!(
            serde_json::from_str::<Option<AutopullActivity>>("null").unwrap(),
            None
        );
    }
}

pub async fn quit_prompt_shown(generation: u64) -> Result<(), String> {
    engine::invoke("quit_prompt_shown", &QuitPromptShownArgs { generation }).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct QuitPromptShownArgs {
    generation: u64,
}

pub async fn quit_confirm() -> Result<(), String> {
    engine::invoke_unit("quit_confirm").await
}

pub async fn quit_cancel() -> Result<(), String> {
    engine::invoke_unit("quit_cancel").await
}

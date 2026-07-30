use leptos::prelude::*;
use quilt_uri::S3PackageUri;

use crate::commands::{self, PausedEvent, PullCheck, PullOutcome};
use crate::components::Notification;
use crate::components::buttons;
use crate::util::host_str;
use crate::util::make_action;
use crate::util::role_denied_hint;

// ── Status banner ──

#[component]
// Leptos props are always passed by value (the generated `Props` builder
// moves them into the component); `status` is only pattern-matched against
// literals, which cannot consume a `String`.
#[allow(clippy::needless_pass_by_value)]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub(super) fn StatusBanner(
    namespace: String,
    status: String,
    /// The package's remote, when it has one. Its catalog answers both
    /// questions this banner asks about the remote: whether there *is* one (and
    /// so which banner to show, and where Login would point), and which
    /// deployment the sync actions here should be attributed to.
    uri: Option<S3PackageUri>,
    /// Why the active role cannot reach this package's bucket, when it
    /// cannot. Outranks every status-driven banner — see
    /// [`remote_state_banner`].
    no_access_reason: Option<String>,
    /// The dry-run pull check for the two-phase Pull affordance, filled in
    /// asynchronously by the parent. `Loading` = still resolving (Pull disabled,
    /// "Checking…"); `Failed` = the dry-run errored (Pull disabled, with a retry
    /// affordance); `Ready` drives the copy and enabled state. Only consulted by
    /// the `behind` arm.
    pull_check: Signal<PullCheck>,
    /// Re-runs the dry-run pull check; wired to the retry affordance shown when
    /// `pull_check` is `Failed`.
    pull_retry: Trigger,
    paused_event: RwSignal<Option<PausedEvent>>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let ns = namespace;
    let host = uri.as_ref().and_then(host_str);

    // A denial and the "unable to check" state are both answers about the
    // remote's reachability rather than about its contents, so one function
    // decides between them — and decides which of the two may offer Login.
    if let Some((description, offers_login)) =
        remote_state_banner(&status, host.is_some(), no_access_reason.as_deref())
    {
        let login_href = offers_login.then(|| {
            let back = format!(
                "/installed-package?namespace={}&filter=unmodified",
                urlencoding::encode(&ns)
            );
            format!(
                "/login?host={}&back={}",
                host.as_deref().unwrap_or_default(),
                urlencoding::encode(&back)
            )
        });
        return with_paused_banner(
            paused_event,
            view! {
                <div class="qui-status">
                    <div class="root">
                        <h2 class="description">{description}</h2>
                        <div class="action">
                            {login_href.map(|href| view! { <buttons::Login href=href /> })}
                        </div>
                    </div>
                </div>
            }
            .into_any(),
        );
    }

    let content = match status.as_str() {
        "ahead" => {
            let ns_for_push = ns.clone();
            let uri_for_push = uri.clone();
            let (push_busy, on_push) = make_action(
                move || {
                    let ns = ns_for_push.clone();
                    let uri = uri_for_push.clone();
                    async move { commands::package_push(ns, uri).await }
                },
                notification,
                Some(ui_locked),
                move || refetch.notify(),
            );
            Some(
                view! {
                    <StatusBannerInner description="Your commits are ahead of the remote">
                        <buttons::Push on_click=on_push busy=push_busy />
                    </StatusBannerInner>
                }
                .into_any(),
            )
        }
        "behind" => {
            let ns_for_pull = ns.clone();
            let uri_for_pull = uri.clone();
            let (pull_busy, on_pull) = make_action(
                move || {
                    let ns = ns_for_pull.clone();
                    let uri = uri_for_pull.clone();
                    async move { commands::package_pull(ns, uri).await }
                },
                notification,
                Some(ui_locked),
                move || refetch.notify(),
            );
            // Two-phase: the banner renders from `status` immediately with a
            // "Checking…" placeholder; the dry-run `PullCheck` (fetched by the
            // parent) then drives the copy and whether Pull is enabled. Pull is
            // disabled while the check is `Loading`, when it `Failed` (a retry
            // is offered), and when the outcome is `Blocked` — a real two-sided
            // conflict — whose message names the conflicting files and points
            // at the merge page. The clean and keeps-local-changes outcomes
            // enable Pull, the latter reassuring the user their local work
            // survives the pull.
            let description = move || behind_description(&pull_check.get());
            let pull_disabled = Signal::derive(move || !pull_check.get().pull_enabled());
            let show_retry = Signal::derive(move || pull_check.get().is_failed());
            Some(
                view! {
                    <div class="qui-status">
                        <div class="root">
                            <h2 class="description">{description}</h2>
                            <div class="action">
                                <buttons::Pull
                                    on_click=on_pull
                                    busy=pull_busy
                                    disabled=pull_disabled
                                />
                                <Show when=move || show_retry.get()>
                                    <buttons::Refresh on_click=move |_| pull_retry.notify() />
                                </Show>
                            </div>
                        </div>
                    </div>
                }
                .into_any(),
            )
        }
        "diverged" => Some(
            view! {
                <StatusBannerInner description="Your commits are detached from the remote">
                    <buttons::Merge namespace=ns.clone() />
                </StatusBannerInner>
            }
            .into_any(),
        ),
        "local" if host.is_some() => {
            let ns_for_push = ns.clone();
            let uri_for_push = uri.clone();
            let (push_busy, on_push) = make_action(
                move || {
                    let ns = ns_for_push.clone();
                    let uri = uri_for_push.clone();
                    async move { commands::package_push(ns, uri).await }
                },
                notification,
                Some(ui_locked),
                move || refetch.notify(),
            );
            Some(
                view! {
                    <StatusBannerInner description="Push to remote">
                        <buttons::Push on_click=on_push busy=push_busy />
                    </StatusBannerInner>
                }
                .into_any(),
            )
        }
        _ => None,
    };

    with_paused_banner(paused_event, content.into_any())
}

/// Autosync `paused` is rendered *in addition to* the upstream-state banner
/// so we can show a reason message the status string alone cannot carry
/// (workflow rejection text, hash mismatch, etc.). When the next non-paused
/// status emit comes in, `paused_event` clears and only the upstream banner
/// remains.
fn with_paused_banner(paused_event: RwSignal<Option<PausedEvent>>, content: AnyView) -> AnyView {
    view! {
        <Show when=move || paused_event.get().is_some()>
            {move || paused_event.get().map(|ev| {
                // Only `reason = "other"` and `"pullConflict"` reach us — the
                // listener in `InstalledPackage` filters everything else out so
                // we don't double-banner Diverged / Behind / Ahead, which are
                // already covered by the status-driven `content` below. The
                // headline + detail are keyed on the reason: a pull conflict
                // names the files and points at the merge page (the same
                // remediation as the manual-pull `Blocked` copy), while every
                // other reason keeps the generic "push manually to resume"
                // guidance with the raw refusal reason as the detail line.
                let (headline, detail) = paused_banner_copy(&ev.reason, ev.message.as_deref());
                view! {
                    <div class="qui-status">
                        <div class="root">
                            <div class="text">
                                <h2 class="description">{headline}</h2>
                                {detail.map(|d| view! { <p class="detail">{d}</p> })}
                            </div>
                        </div>
                    </div>
                }
            })}
        </Show>
        {content}
    }
    .into_any()
}

/// The banner for what we know about the remote's *reachability*, or `None`
/// when the status says nothing about it and the status-driven banners apply.
///
/// Returns the description and whether Login is the remedy on offer. The two
/// cases used to be one: every status failure, denials included, collapsed
/// into `error`, whose "Unable to check remote status" copy comes with a
/// Login button. For a denial that button is a loop with no exit — the
/// session is healthy, so the re-vend hands back the very role that was
/// refused. So a known denial outranks the error state, states the fact in
/// the roster's words, and offers nothing.
fn remote_state_banner(
    status: &str,
    has_host: bool,
    no_access_reason: Option<&str>,
) -> Option<(String, bool)> {
    if let Some(reason) = no_access_reason {
        return Some((reason.to_string(), false));
    }
    if status != "error" {
        return None;
    }
    Some(if has_host {
        ("Unable to check remote status".to_string(), true)
    } else {
        ("No remote configured".to_string(), false)
    })
}

/// Headline + optional detail line for the autosync paused banner, keyed on the
/// pause reason. A `pullConflict` names the conflicting files and points at the
/// merge page — the same remediation the manual-pull `Blocked` copy gives —
/// because "push manually to resume" is the wrong fix for a pull conflict. A
/// `roleDenied` is the same story for a different reason: a manual push runs
/// under the very role that was refused, so only a role switch clears it.
/// Every other reason keeps the generic guidance with the raw refusal reason as
/// the detail line.
fn paused_banner_copy(reason: &str, message: Option<&str>) -> (String, Option<String>) {
    const GENERIC: &str = "Autosync paused. Resolve the issue, then push manually to resume.";
    match reason {
        "roleDenied" => (role_denied_hint(message), None),
        "pullConflict" => {
            let headline = match message {
                Some(files) if !files.is_empty() => format!(
                    "Conflicts in {files}. Commit your changes to resolve them on the merge page."
                ),
                _ => "Pull conflict. Commit your changes to resolve it on the merge page."
                    .to_string(),
            };
            (headline, None)
        }
        _ => (GENERIC.to_string(), message.map(str::to_string)),
    }
}

/// The `behind`-arm banner description for the dry-run pull check. `Loading` is
/// the genuine in-flight state (Pull disabled, "Checking…"); `Failed` is an
/// honest fetch-error state (Pull disabled, a retry offered); `Ready` defers to
/// [`outcome_description`].
fn behind_description(check: &PullCheck) -> String {
    match check {
        PullCheck::Loading => "Checking for updates\u{2026}".to_string(),
        PullCheck::Failed => "Couldn't check for updates.".to_string(),
        PullCheck::Ready(outcome) => outcome_description(outcome),
    }
}

/// The banner copy for a resolved dry-run outcome. `Blocked` names the
/// conflicting files and points at the merge page; `KeepsLocalChanges`
/// reassures the user their local work survives; everything else states there
/// are newer revisions.
fn outcome_description(outcome: &PullOutcome) -> String {
    match outcome {
        PullOutcome::Blocked { conflicts } => format!(
            "Conflicts in {}. Commit your changes to resolve them on the merge page.",
            conflicts.join(", ")
        ),
        PullOutcome::KeepsLocalChanges { .. } => {
            "The remote has newer revisions. Your local changes are safe — pulling keeps them."
                .to_string()
        }
        _ => "The remote has newer revisions.".to_string(),
    }
}

#[component]
fn StatusBannerInner(description: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="qui-status">
            <div class="root">
                <h2 class="description">{description}</h2>
                <div class="action">
                    {children()}
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{behind_description, paused_banner_copy, remote_state_banner};
    use crate::commands::{PullCheck, PullOutcome};

    const DENIED: &str = "Current role ReadOnly has no access to this bucket";

    /// The regression this replaces: a denial arrived as the `error` status,
    /// and the page told the user to sign in again. The re-vend returns the
    /// same refused role, so that button is a loop with no exit.
    #[test]
    fn a_denial_states_the_fact_and_offers_no_login() {
        assert_eq!(
            remote_state_banner("error", true, Some(DENIED)),
            Some((DENIED.to_string(), false))
        );
    }

    /// The mark can arrive on a row whose cached status is perfectly fine —
    /// the page still has to explain why the remote state is not being
    /// refreshed.
    #[test]
    fn a_denial_outranks_a_healthy_status_too() {
        assert_eq!(
            remote_state_banner("up_to_date", true, Some(DENIED)),
            Some((DENIED.to_string(), false))
        );
    }

    /// A genuine failure keeps its Login route — only a denial loses it.
    #[test]
    fn an_unexplained_error_still_offers_login() {
        assert_eq!(
            remote_state_banner("error", true, None),
            Some(("Unable to check remote status".to_string(), true))
        );
    }

    #[test]
    fn an_error_without_a_host_has_no_remote_to_sign_in_to() {
        assert_eq!(
            remote_state_banner("error", false, None),
            Some(("No remote configured".to_string(), false))
        );
    }

    #[test]
    fn a_healthy_status_leaves_the_banner_to_the_status_arms() {
        assert_eq!(remote_state_banner("behind", true, None), None);
    }

    #[test]
    fn paused_other_shows_generic_guidance_and_raw_reason_detail() {
        assert_eq!(
            paused_banner_copy("other", Some("workflow rejected metadata")),
            (
                "Autosync paused. Resolve the issue, then push manually to resume.".to_string(),
                Some("workflow rejected metadata".to_string()),
            )
        );
    }

    #[test]
    fn paused_pull_conflict_names_files_and_points_at_merge() {
        // Same copy shape as the manual-pull `Blocked` banner, so the
        // autosync-on and status-behind paths read identically.
        assert_eq!(
            paused_banner_copy("pullConflict", Some("a.txt, b.txt")),
            (
                "Conflicts in a.txt, b.txt. Commit your changes to resolve them on the merge page."
                    .to_string(),
                None,
            )
        );
    }

    #[test]
    fn paused_pull_conflict_falls_back_without_files() {
        assert_eq!(
            paused_banner_copy("pullConflict", None),
            (
                "Pull conflict. Commit your changes to resolve it on the merge page.".to_string(),
                None,
            )
        );
    }

    /// The generic "push manually to resume" guidance is actively wrong for a
    /// denial: the manual push runs under the same refused role. The banner
    /// must name the role and the switch instead.
    #[test]
    fn paused_role_denied_names_the_role_and_the_switch() {
        assert_eq!(
            paused_banner_copy("roleDenied", Some("ReadOnly")),
            (
                "Current role ReadOnly has no access to this bucket. \
                 Switch role to resume autosync."
                    .to_string(),
                None,
            )
        );
    }

    #[test]
    fn paused_role_denied_falls_back_without_a_role_name() {
        assert_eq!(
            paused_banner_copy("roleDenied", None),
            (
                "The active role has no access to this bucket. \
                 Switch role to resume autosync."
                    .to_string(),
                None,
            )
        );
    }

    #[test]
    fn loading_check_shows_checking_placeholder() {
        assert_eq!(
            behind_description(&PullCheck::Loading),
            "Checking for updates\u{2026}"
        );
    }

    #[test]
    fn failed_check_shows_honest_error_and_keeps_pull_disabled() {
        // The error state is distinct from loading — an honest failure the
        // retry affordance pairs with — and Pull stays disabled (fail-safe).
        assert_eq!(
            behind_description(&PullCheck::Failed),
            "Couldn't check for updates."
        );
        assert!(!PullCheck::Failed.pull_enabled());
        assert!(PullCheck::Failed.is_failed());
        assert!(!PullCheck::Loading.pull_enabled());
        assert!(!PullCheck::Loading.is_failed());
    }

    #[test]
    fn clean_update_states_newer_revisions() {
        assert_eq!(
            behind_description(&PullCheck::Ready(PullOutcome::CleanUpdate)),
            "The remote has newer revisions."
        );
        assert!(PullCheck::Ready(PullOutcome::CleanUpdate).pull_enabled());
    }

    #[test]
    fn keeps_local_changes_reassures_local_work_is_safe() {
        let check = PullCheck::Ready(PullOutcome::KeepsLocalChanges {
            added: vec!["a.txt".to_string()],
            modified: vec![],
            removed: vec![],
        });
        assert_eq!(
            behind_description(&check),
            "The remote has newer revisions. Your local changes are safe — pulling keeps them."
        );
        assert!(check.pull_enabled());
    }

    #[test]
    fn blocked_names_conflicts_and_keeps_pull_disabled() {
        let check = PullCheck::Ready(PullOutcome::Blocked {
            conflicts: vec!["a.txt".to_string(), "b.txt".to_string()],
        });
        assert_eq!(
            behind_description(&check),
            "Conflicts in a.txt, b.txt. Commit your changes to resolve them on the merge page."
        );
        assert!(!check.pull_enabled());
        assert!(!check.is_failed());
    }
}

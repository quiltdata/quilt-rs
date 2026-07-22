use leptos::prelude::*;

use crate::commands::{self, PausedEvent, PullCheck, PullOutcome};
use crate::components::Notification;
use crate::components::buttons;
use crate::util::make_action;

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
    origin_host: Option<String>,
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
    let host = origin_host;

    let content = match status.as_str() {
        "ahead" => {
            let ns_for_push = ns.clone();
            let (push_busy, on_push) = make_action(
                move || {
                    let ns = ns_for_push.clone();
                    async move { commands::package_push(ns).await }
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
            let (pull_busy, on_pull) = make_action(
                move || {
                    let ns = ns_for_pull.clone();
                    async move { commands::package_pull(ns).await }
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
        "error" => match host {
            Some(ref h) => {
                let back = format!(
                    "/installed-package?namespace={}&filter=unmodified",
                    urlencoding::encode(&ns)
                );
                let login_href = format!("/login?host={}&back={}", h, urlencoding::encode(&back));
                Some(
                    view! {
                        <StatusBannerInner description="Unable to check remote status">
                            <buttons::Login href=login_href />
                        </StatusBannerInner>
                    }
                    .into_any(),
                )
            }
            None => Some(
                view! {
                    <StatusBannerInner description="No remote configured">
                        <span></span>
                    </StatusBannerInner>
                }
                .into_any(),
            ),
        },
        "local" if host.is_some() => {
            let ns_for_push = ns.clone();
            let (push_busy, on_push) = make_action(
                move || {
                    let ns = ns_for_push.clone();
                    async move { commands::package_push(ns).await }
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

    // Autosync `paused` is rendered *in addition to* the
    // upstream-state banner so we can show a reason message the
    // status string alone cannot carry (workflow rejection text, hash
    // mismatch, etc.). When the next non-paused status emit comes in,
    // `paused_event` clears and only the upstream banner remains.
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
}

/// Headline + optional detail line for the autosync paused banner, keyed on the
/// pause reason. A `pullConflict` names the conflicting files and points at the
/// merge page — the same remediation the manual-pull `Blocked` copy gives —
/// because "push manually to resume" is the wrong fix for a pull conflict.
/// Every other reason keeps the generic guidance with the raw refusal reason as
/// the detail line.
fn paused_banner_copy(reason: &str, message: Option<&str>) -> (String, Option<String>) {
    const GENERIC: &str = "Autosync paused. Resolve the issue, then push manually to resume.";
    match reason {
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
    use super::{behind_description, paused_banner_copy};
    use crate::commands::{PullCheck, PullOutcome};

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

use leptos::prelude::*;

use crate::commands::{self, PausedEvent, PullOutcome};
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
    /// The dry-run pull outcome for the two-phase Pull affordance, filled in
    /// asynchronously by the parent. `None` = still resolving (or failed):
    /// Pull renders disabled with a "Checking…" placeholder. Only consulted by
    /// the `behind` arm.
    pull_outcome: Signal<Option<PullOutcome>>,
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
            // "Checking…" placeholder; the dry-run `PullOutcome` (fetched by
            // the parent) then drives both the copy and whether Pull is
            // enabled. Pull is disabled while the outcome is unknown and when
            // it is `Blocked` — a real two-sided conflict — whose message names
            // the conflicting files and points at the merge page. The clean and
            // keeps-local-changes outcomes enable Pull, the latter reassuring
            // the user their local work survives the pull.
            let description = move || behind_description(pull_outcome.get().as_ref());
            let pull_disabled =
                Signal::derive(move || !pull_outcome.get().is_some_and(|o| o.is_pullable()));
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
                // Only `reason = "other"` reaches us — the listener in
                // `InstalledPackage` filters everything else out so we
                // don't double-banner Diverged / Behind / Ahead, which
                // are already covered by the status-driven `content`
                // below. `message` carries just the raw refusal reason;
                // the guidance line ("resolve, then push manually to
                // resume") is presentation, added here.
                let reason = ev.message;
                view! {
                    <div class="qui-status">
                        <div class="root">
                            <div class="text">
                                <h2 class="description">
                                    "Autosync paused. Resolve the issue, then push manually to resume."
                                </h2>
                                {reason.map(|r| view! { <p class="detail">{r}</p> })}
                            </div>
                        </div>
                    </div>
                }
            })}
        </Show>
        {content}
    }
}

/// The `behind`-arm banner description for a (possibly still-loading) pull
/// outcome. `None` = the dry-run outcome has not resolved yet (or failed to),
/// so the copy invites the user to wait while Pull stays disabled.
fn behind_description(outcome: Option<&PullOutcome>) -> String {
    match outcome {
        None => "Checking for updates\u{2026}".to_string(),
        Some(PullOutcome::Blocked { conflicts }) => format!(
            "Conflicts in {}. Commit your changes to resolve them on the merge page.",
            conflicts.join(", ")
        ),
        Some(PullOutcome::KeepsLocalChanges { .. }) => {
            "The remote has newer revisions. Your local changes are safe — pulling keeps them."
                .to_string()
        }
        Some(_) => "The remote has newer revisions.".to_string(),
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
    use super::behind_description;
    use crate::commands::PullOutcome;

    #[test]
    fn loading_outcome_shows_checking_placeholder() {
        assert_eq!(behind_description(None), "Checking for updates\u{2026}");
    }

    #[test]
    fn clean_update_states_newer_revisions() {
        assert_eq!(
            behind_description(Some(&PullOutcome::CleanUpdate)),
            "The remote has newer revisions."
        );
    }

    #[test]
    fn keeps_local_changes_reassures_local_work_is_safe() {
        let outcome = PullOutcome::KeepsLocalChanges {
            added: vec!["a.txt".to_string()],
            modified: vec![],
            removed: vec![],
        };
        assert_eq!(
            behind_description(Some(&outcome)),
            "The remote has newer revisions. Your local changes are safe — pulling keeps them."
        );
    }

    #[test]
    fn blocked_names_conflicts_and_points_at_merge() {
        let outcome = PullOutcome::Blocked {
            conflicts: vec!["a.txt".to_string(), "b.txt".to_string()],
        };
        assert_eq!(
            behind_description(Some(&outcome)),
            "Conflicts in a.txt, b.txt. Commit your changes to resolve them on the merge page."
        );
    }
}

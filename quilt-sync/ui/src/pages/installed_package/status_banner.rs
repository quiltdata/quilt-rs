use leptos::prelude::*;

use crate::commands::{self, PausedEvent};
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
    has_changes: bool,
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
            // The old wording assumed local commits ("Your commits are
            // behind the remote") but `Behind` is reachable from a
            // pristine install + remote movement — there may be no
            // commits at all. State the actual fact: the remote has
            // newer revisions. If working-tree changes block pull, say
            // so up-front (in the banner, not in a hover popover) so
            // autosync's reason for not auto-pulling is visible.
            let description: &'static str = if has_changes {
                "The remote has newer revisions. Commit or discard your local changes to pull."
            } else {
                "The remote has newer revisions."
            };
            Some(
                view! {
                    <StatusBannerInner description=description>
                        <buttons::Pull on_click=on_pull busy=pull_busy disabled=has_changes />
                    </StatusBannerInner>
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

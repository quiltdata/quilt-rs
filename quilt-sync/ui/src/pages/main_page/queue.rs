//! §4.3: the attention queue, derived and drawn. It has no payload of its own —
//! given the resolved package list and the host facts, the grouping, the
//! counts and the order are all computed here, and [`QueueRegion`] draws what
//! that computation produces.

use std::collections::HashMap;
use std::collections::HashSet;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use super::accounts::sign_in_href;
use crate::commands::AccountHostData;
use crate::commands::MainPagePackageData;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::CauseRow;
use crate::kit::PackageState;
use crate::kit::QueueRow;
use crate::kit::Site;
use crate::kit::ZeroLine;
use crate::kit::render;

/// One row in the queue, in draw order: a cause shared by several packages,
/// or a package needing its own decision.
#[derive(Clone, Debug)]
pub enum QueueItem {
    /// Several packages collapsed under one explanation (R2, R3). `members`
    /// is the namespaces of the packages the cause holds, in the input's own
    /// order — its length is the count `CauseRow` renders; no count is ever
    /// passed in or written separately.
    Cause {
        text: String,
        action: CauseAction,
        members: Vec<String>,
    },
    /// A package needing its own decision. Carries the `PackageState` itself,
    /// not its words: Task 3 renders it with `render(&state, Site::QueueRow)`,
    /// the one exception being a cause's own composed text (see
    /// [`derive_queue`]).
    Package {
        namespace: String,
        // Read by `QueueRegion` to render — the one exception being a cause's
        // own composed text (see [`derive_queue`]).
        state: PackageState,
    },
}

/// What a cause's trailing slot offers.
#[derive(Clone, Debug)]
pub enum CauseAction {
    /// `[Sign in]`, targeting this host.
    SignIn { host: String },
    /// No control here — switching role is host-scoped, so it belongs to the
    /// host row in the Accounts card; the trailing slot points at it instead
    /// of duplicating it.
    ///
    /// A unit variant, carrying no host: the slot is a fixed sentence rather
    /// than a control, and the host a denial names is already in the cause's
    /// own `text` (see [`role_denied_text`]). A field nothing renders would be
    /// the same fact stored twice, told apart only by which copy a later
    /// change forgot.
    SwitchRole,
}

/// The queue, derived. Section 4.3: it has no payload — given the resolved
/// package list and the host facts, everything below is computed here.
pub fn derive_queue(packages: &[MainPagePackageData], hosts: &[AccountHostData]) -> Vec<QueueItem> {
    let signed_out: HashSet<&str> = hosts
        .iter()
        .filter(|h| !h.signed_in)
        .map(|h| h.host.as_str())
        .collect();

    let mut signed_out_groups: HashMap<&str, Vec<&MainPagePackageData>> = HashMap::new();
    let mut role_denied_groups: HashMap<&str, Vec<&MainPagePackageData>> = HashMap::new();
    let mut rows: Vec<&MainPagePackageData> = Vec::new();

    // Two passes rather than one: a package's membership of a shared cause is
    // decided by the join, and only what the join rejects becomes its own row.
    for package in packages {
        // R3: Unknown alone is not enough — it is serde's catch-all for a
        // state this build could not read, of which a signed-out host is only
        // one cause. The join against `signed_out` is the other half.
        let is_signed_out = package.state == PackageState::Unknown
            && package
                .host
                .as_deref()
                .is_some_and(|host| signed_out.contains(host));
        if is_signed_out {
            let host = package
                .host
                .as_deref()
                .expect("checked by is_some_and above");
            signed_out_groups.entry(host).or_default().push(package);
            continue;
        }

        // R2: grouped by bucket, never by host — one host can hold both
        // readable and unreadable buckets. A denial with no bucket to name
        // becomes its own row instead (ruling 3): a cause keyed on a bucket
        // cannot name one that is absent.
        if matches!(package.state, PackageState::RoleDenied { .. })
            && let Some(bucket) = package.bucket.as_deref()
        {
            role_denied_groups.entry(bucket).or_default().push(package);
            continue;
        }

        if package.state != PackageState::Latest {
            rows.push(package);
        }
    }

    // (rank, text, item) so the final sort is by §5's cause rank first and
    // the cause text second — never hash-iteration order, which is all a
    // `HashMap`'s own order would give us.
    let mut ranked_causes: Vec<(u8, String, QueueItem)> = Vec::new();

    for (bucket, members) in role_denied_groups {
        // One bucket is one denial: every member shares the same bucket by
        // construction, and the role and host it names come from the group's
        // first member in the input's own order. A difference among members
        // here would be a backend inconsistency, not a case to render twice.
        let first = members[0];
        let PackageState::RoleDenied { role } = &first.state else {
            unreachable!("role_denied_groups only ever collects RoleDenied packages")
        };
        let text = role_denied_text(role.as_deref(), first.host.as_deref(), bucket);
        ranked_causes.push((
            0, // §5 row 1: role-denied sorts before signed-out.
            text.clone(),
            QueueItem::Cause {
                text,
                action: CauseAction::SwitchRole,
                members: members.iter().map(|p| p.namespace.clone()).collect(),
            },
        ));
    }

    for (host, members) in signed_out_groups {
        let text = format!("Signed out from {host}");
        ranked_causes.push((
            4, // §5 row 4: signed-out is the attributable half of "error".
            text.clone(),
            QueueItem::Cause {
                text,
                action: CauseAction::SignIn {
                    host: host.to_string(),
                },
                members: members.iter().map(|p| p.namespace.clone()).collect(),
            },
        ));
    }

    ranked_causes.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let causes = ranked_causes.into_iter().map(|(_, _, item)| item);

    rows.sort_by_key(|p| precedence(&p.state));
    let rows = rows.into_iter().map(|p| QueueItem::Package {
        namespace: p.namespace.clone(),
        state: p.state.clone(),
    });

    causes.chain(rows).collect()
}

/// Ruling 5's cause text: drop the `as {role}` clause when the role is
/// unknown, and the `on {host}` clause when the package has no host. The
/// count is never part of this string — `CauseRow` renders `— N packages`
/// itself from `members.len()`.
fn role_denied_text(role: Option<&str>, host: Option<&str>, bucket: &str) -> String {
    match (role, host) {
        (Some(role), Some(host)) => format!("No access as {role} on {host} in s3://{bucket}"),
        (Some(role), None) => format!("No access as {role} in s3://{bucket}"),
        (None, Some(host)) => format!("No access on {host} in s3://{bucket}"),
        (None, None) => format!("No access in s3://{bucket}"),
    }
}

/// Section 5's lattice, as a sort key. Lower sorts first: pull-conflict above
/// error above diverged above behind above "has changes" above no-remote
/// above unpublished. `Unknown` takes the "error" rank — of which the
/// signed-out group is the attributable half, so what reaches this function
/// as `Unknown` is a state this build could not otherwise explain.
/// `RoleDenied` sorts last only to keep this match total: R2 groups every
/// denial by bucket, so a `RoleDenied` package never reaches `rows` unless
/// its bucket is absent, and a denial is not unimportant.
fn precedence(state: &PackageState) -> u8 {
    match state {
        PackageState::PullConflict { .. } => 0,
        PackageState::Unknown => 1,
        PackageState::Diverged => 2,
        PackageState::Behind => 3,
        PackageState::PendingChanges { .. } | PackageState::PendingCommit => 4,
        PackageState::NoRemote => 5,
        PackageState::Unpublished => 6,
        PackageState::RoleDenied { .. } => 7,
        PackageState::Latest => unreachable!("Latest never enters either collection"),
    }
}

/// `Everything is Latest — 43 packages`. One string, because the singular case
/// is not a plural rule `ZeroLine` can apply.
fn zero_line_text(total: usize) -> String {
    if total == 1 {
        "Everything is Latest — 1 package".to_string()
    } else {
        format!("Everything is Latest — {total} packages")
    }
}

/// The one page that can act on a package row's `Rendered.action` label —
/// every action is a navigation, never a mutation. `Get latest` and `Choose S3
/// bucket` have no page of their own: in v1 they are `buttons::Pull`
/// (`pages/installed_package/status_banner.rs:138`) and `buttons::SetRemote`
/// (`pages/installed_package/toolbar.rs:99`), both living on the package's own
/// page, so landing there is the honest answer rather than inventing a command.
/// Total over the labels `render` ever hands back for a state that has one —
/// `RoleDenied`, `Unknown` and `Latest` never reach here because their action
/// is `None`.
fn action_href(label: &str, namespace: &str) -> String {
    match label {
        "Publish" => format!("/commit?namespace={namespace}"), // content.rs:195
        "Resolve" => format!("/merge?namespace={namespace}"),  // components/buttons/merge.rs:10
        "Get latest" | "Choose S3 bucket" => {
            // main_page.rs:151, the same shape installed_packages_list.rs:445 uses.
            format!("/installed-package?namespace={namespace}&filter=unmodified")
        }
        other => unreachable!("render() never offers the action {other:?}"),
    }
}

/// A package row's `[Publish]` / `[Resolve]` / `[Get latest]` / `[Choose S3
/// bucket]` — whichever `render`'s `Rendered.action` names. The click
/// navigates; there is no mutation here.
fn package_action(
    label: &'static str,
    namespace: &str,
    navigate: impl Fn(&str, NavigateOptions) + Clone + 'static,
) -> AnyView {
    let target = action_href(label, namespace);
    view! { <Button on_click=move |_| navigate(&target, NavigateOptions::default())>{label}</Button> }
        .into_any()
}

/// A cause's trailing slot: `[Sign in]` for a signed-out host, or the pointer
/// line for a role denial — never both, and never a `[Switch role]` (ruling 3):
/// that control is host-scoped and lives on the Accounts card's host row.
fn cause_trailing(
    action: &CauseAction,
    navigate: impl Fn(&str, NavigateOptions) + Clone + 'static,
) -> AnyView {
    match action {
        CauseAction::SignIn { host } => {
            let target = sign_in_href(host);
            view! {
                <Button on_click=move |_| navigate(&target, NavigateOptions::default())>
                    "Sign in"
                </Button>
            }
            .into_any()
        }
        CauseAction::SwitchRole => view! { "Change your role in Accounts, above." }.into_any(),
    }
}

/// §4.3, drawn: shared causes first, each with its count and an expander, then
/// one row per package in precedence order, each beside the one thing to do
/// about it — and a single line when nothing needs a decision. No payload of
/// its own (global constraint): the caller resolves both lists and hands them
/// in.
#[component]
// Owned, not borrowed, matching `components/buttons/merge.rs`'s own `Merge`:
// a prop only borrowed inside the body still has to be owned by the caller,
// since a component's arguments outlive the call that builds them.
#[allow(clippy::needless_pass_by_value)]
pub fn QueueRegion(
    packages: Vec<MainPagePackageData>,
    hosts: Vec<AccountHostData>,
) -> impl IntoView {
    let total = packages.len();
    let items = derive_queue(&packages, &hosts);
    if items.is_empty() {
        // Acceptance criterion 8: one line, not an empty state — with autosync
        // working this is the common case, and a full-height panel here would
        // push the package list below the fold to announce that nothing is wrong.
        return view! { <ZeroLine text=zero_line_text(total) /> }.into_any();
    }

    let navigate = use_navigate();
    // Derived from the rows rendered, never written by hand — a `Cause`'s count
    // is its members, a `Package` is one of itself.
    let count: usize = items
        .iter()
        .map(|item| match item {
            QueueItem::Cause { members, .. } => members.len(),
            QueueItem::Package { .. } => 1,
        })
        .sum();

    view! {
        // One wrapper child, so `Card`'s between-children hairline does not
        // fire: a queue is a list of decisions, and dividing every row would
        // make it read as a table.
        <Card title="Needs your attention" count=count>
            <div>
                {items
                    .into_iter()
                    .map(|item| match item {
                        QueueItem::Cause { text, action, members } => {
                            let expanded = RwSignal::new(false);
                            let member_count = members.len();
                            let trailing = cause_trailing(&action, navigate.clone());
                            view! {
                                <CauseRow
                                    text=text
                                    count=member_count
                                    expanded=expanded
                                    trailing=trailing
                                />
                                <Show when=move || expanded.get()>
                                    {members
                                        .iter()
                                        .map(|namespace| {
                                            view! { <QueueRow namespace=namespace.clone() sub=true /> }
                                        })
                                        .collect_view()}
                                </Show>
                            }
                                .into_any()
                        }
                        QueueItem::Package { namespace, state } => {
                            let rendered = render(&state, Site::QueueRow);
                            // `Rendered.action` is `Option<&'static str>` — `None` renders
                            // a row with no button, the honest answer for a state the app
                            // has no operation to fix. Not invented here.
                            if let Some(label) = rendered.action {
                                let action = package_action(label, &namespace, navigate.clone());
                                view! {
                                    <QueueRow
                                        namespace=namespace
                                        state=rendered.words
                                        tone=rendered.tone
                                        action=action
                                    />
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <QueueRow namespace=namespace state=rendered.words tone=rendered.tone />
                                }
                                    .into_any()
                            }
                        }
                    })
                    .collect_view()}
            </div>
        </Card>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AccountHostData;
    use crate::commands::MainPagePackageData;
    use crate::kit::PackageState;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn pkg(namespace: &str, state: PackageState, host: Option<&str>) -> MainPagePackageData {
        MainPagePackageData {
            namespace: namespace.to_string(),
            state,
            changed_at: None,
            bucket: None,
            host: host.map(str::to_string),
            provisional: false,
            role_switch_host: None,
        }
    }

    fn pkg_in_bucket(
        namespace: &str,
        state: PackageState,
        host: &str,
        bucket: &str,
    ) -> MainPagePackageData {
        MainPagePackageData {
            namespace: namespace.to_string(),
            state,
            changed_at: None,
            bucket: Some(bucket.to_string()),
            host: Some(host.to_string()),
            provisional: false,
            role_switch_host: None,
        }
    }

    fn host(name: &str, signed_in: bool) -> AccountHostData {
        AccountHostData {
            host: name.to_string(),
            signed_in,
            current_role: None,
            roles: Vec::new(),
            provisional: false,
        }
    }

    #[wasm_bindgen_test]
    fn a_latest_package_never_reaches_the_queue() {
        // The queue is what needs a decision. Everything else is the list's job.
        let items = derive_queue(
            &[pkg("a/b", PackageState::Latest, Some("h.io"))],
            &[host("h.io", true)],
        );
        assert!(items.is_empty());
    }

    #[wasm_bindgen_test]
    fn signed_out_packages_collapse_into_one_cause_naming_the_host() {
        // R3. Unknown state AND a host the accounts payload says is signed out.
        // Without the grouping, a signed-out host with 11 packages buries the three
        // problems that need individual decisions.
        let items = derive_queue(
            &[
                pkg("a/one", PackageState::Unknown, Some("custom.registry.io")),
                pkg("a/two", PackageState::Unknown, Some("custom.registry.io")),
                pkg("b/three", PackageState::Behind, Some("custom.registry.io")),
            ],
            &[host("custom.registry.io", false)],
        );

        match &items[0] {
            QueueItem::Cause {
                text,
                action,
                members,
            } => {
                assert_eq!(text, "Signed out from custom.registry.io");
                assert_eq!(members.len(), 2, "the two Unknown ones, not the Behind one");
                assert!(
                    matches!(action, CauseAction::SignIn { host } if host == "custom.registry.io"),
                    "a wrong host wired into [Sign in] must fail this: {action:?}"
                );
            }
            other @ QueueItem::Package { .. } => panic!("expected a cause first, got {other:?}"),
        }
        assert!(
            matches!(&items[1], QueueItem::Package { namespace, .. } if namespace == "b/three"),
            "a package with its own state is not swept into the cause"
        );
    }

    #[wasm_bindgen_test]
    fn an_unknown_package_on_a_signed_in_host_is_not_signed_out() {
        // R3's other half, and the one that would tell a signed-in user to sign in.
        // Unknown is also serde's catch-all, so it means "we could not tell" — of
        // which a logout is one cause among several.
        let items = derive_queue(
            &[pkg("a/one", PackageState::Unknown, Some("h.io"))],
            &[host("h.io", true)],
        );
        assert!(
            items.iter().all(|i| !matches!(i, QueueItem::Cause { .. })),
            "no cause: the session is fine"
        );
        assert_eq!(
            items.len(),
            1,
            "it is still a row — we could not tell, and that is worth saying"
        );
    }

    #[wasm_bindgen_test]
    fn role_denied_groups_by_bucket_not_by_host() {
        // R2. One host can hold both readable and unreadable buckets, so grouping
        // this by host would put packages the user CAN read inside a group saying
        // they cannot.
        let items = derive_queue(
            &[
                pkg_in_bucket(
                    "a/one",
                    PackageState::RoleDenied {
                        role: Some("analyst".into()),
                    },
                    "h.io",
                    "team-bucket",
                ),
                pkg_in_bucket(
                    "a/two",
                    PackageState::RoleDenied {
                        role: Some("analyst".into()),
                    },
                    "h.io",
                    "team-bucket",
                ),
                pkg_in_bucket(
                    "a/three",
                    PackageState::RoleDenied {
                        role: Some("analyst".into()),
                    },
                    "h.io",
                    "other-bucket",
                ),
            ],
            &[host("h.io", true)],
        );
        let causes: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, QueueItem::Cause { .. }))
            .collect();
        assert_eq!(causes.len(), 2, "two buckets, two causes, one host");
        for cause in causes {
            let QueueItem::Cause { text, .. } = cause else {
                unreachable!("filtered to causes above")
            };
            // The host is named in the cause's own words, which is where a user
            // reads it: the trailing slot is a fixed sentence pointing at the
            // Accounts card, so nothing else in the row can carry it.
            assert!(
                text.contains("on h.io"),
                "a denial has to name the host it is on: {text}"
            );
        }
    }

    #[wasm_bindgen_test]
    fn a_pause_outranks_a_signed_out_host() {
        // R4. The backend already resolved this into the state, so a conflicted
        // package on a signed-out host arrives as PullConflict, never Unknown, and
        // the signed-out join cannot see it. This test pins that it stays true.
        let items = derive_queue(
            &[pkg(
                "a/one",
                PackageState::PullConflict {
                    files: vec!["f.csv".into()],
                },
                Some("h.io"),
            )],
            &[host("h.io", false)],
        );
        assert_eq!(items.len(), 1);
        assert!(
            matches!(&items[0], QueueItem::Package { .. }),
            "its own row, not inside the signed-out group"
        );
    }

    #[wasm_bindgen_test]
    fn a_cause_of_one_is_still_a_cause() {
        // CauseRow renders "1 package" singular deliberately: a cause affecting one
        // package is still worth stating once rather than twice.
        let items = derive_queue(
            &[pkg("a/one", PackageState::Unknown, Some("h.io"))],
            &[host("h.io", false)],
        );
        assert!(matches!(&items[0], QueueItem::Cause { members, .. } if members.len() == 1));
    }

    #[wasm_bindgen_test]
    fn causes_come_before_packages_and_packages_follow_the_lattice() {
        // Section 5's order: shared causes first, then per-package rows in
        // precedence order. Danger before Attention before Neutral.
        let items = derive_queue(
            &[
                pkg("a/behind", PackageState::Behind, Some("h.io")),
                pkg(
                    "a/conflict",
                    PackageState::PullConflict {
                        files: vec!["f".into()],
                    },
                    Some("h.io"),
                ),
                pkg("a/out", PackageState::Unknown, Some("gone.io")),
            ],
            &[host("h.io", true), host("gone.io", false)],
        );
        let shape: Vec<&str> = items
            .iter()
            .map(|i| match i {
                QueueItem::Cause { .. } => "cause",
                QueueItem::Package { namespace, .. } => namespace.as_str(),
            })
            .collect();
        assert_eq!(shape, vec!["cause", "a/conflict", "a/behind"]);
    }

    #[wasm_bindgen_test]
    fn a_local_only_package_is_never_grouped_by_host() {
        // Task 1's `host: None`. Without this it would group under a host named "".
        let items = derive_queue(
            &[pkg("local/thing", PackageState::Unpublished, None)],
            &[host("h.io", false)],
        );
        assert!(matches!(&items[0], QueueItem::Package { .. }));
    }

    // §§ QueueRegion — the drawn region, mounted inside a `Router` because the
    // actions navigate.

    // `Router`'s children are `TypedChildren`, which boxes as `dyn FnOnce() -> _
    // + Send` — harmless on wasm's single thread, but it means `f` must carry
    // the bound even though nothing here is ever sent across one.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + Send + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), move || {
            view! { <leptos_router::components::Router>{f()}</leptos_router::components::Router> }
        })
        .forget();
        container.into()
    }

    /// `autosync.rs`'s pattern: `dyn_into` to the concrete element, then the
    /// DOM's own `.click()` — a real click, not a synthesized event.
    fn click(el: &web_sys::Element) {
        let el: web_sys::HtmlElement = el.clone().dyn_into().unwrap();
        el.click();
    }

    fn all_latest(n: usize) -> Vec<MainPagePackageData> {
        (0..n)
            .map(|i| pkg(&format!("pkg/{i}"), PackageState::Latest, Some("h.io")))
            .collect()
    }

    fn one_signed_in() -> Vec<AccountHostData> {
        vec![host("h.io", true)]
    }

    fn two_signed_out() -> Vec<MainPagePackageData> {
        vec![
            pkg("a/one", PackageState::Unknown, Some("custom.registry.io")),
            pkg("a/two", PackageState::Unknown, Some("custom.registry.io")),
        ]
    }

    fn one_signed_out() -> Vec<AccountHostData> {
        vec![host("custom.registry.io", false)]
    }

    fn one_role_denied() -> Vec<MainPagePackageData> {
        vec![pkg_in_bucket(
            "a/one",
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
            "custom.registry.io",
            "team-bucket",
        )]
    }

    fn one_behind() -> Vec<MainPagePackageData> {
        vec![pkg("a/one", PackageState::Behind, Some("h.io"))]
    }

    /// `Unknown` on a signed-in host: R3's other half, so it is its own row
    /// rather than swept into a signed-out cause — and `render` gives it no
    /// action, unlike `one_behind`.
    fn one_unknown_signed_in() -> Vec<MainPagePackageData> {
        vec![pkg("a/one", PackageState::Unknown, Some("h.io"))]
    }

    #[wasm_bindgen_test]
    fn a_healthy_queue_is_one_line_and_not_a_region() {
        // Acceptance criterion 8, and ZeroLine's own doc: with autosync working
        // this is the common case, and a full-height empty state here would push
        // the package list below the fold to announce that nothing is wrong.
        let el = mount(|| view! { <QueueRegion packages=all_latest(43) hosts=one_signed_in() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("Everything is Latest"), "got: {text}");
        assert!(
            text.contains("43 packages"),
            "the count is derived from the rows: {text}"
        );
        assert!(
            !text.contains("Needs your attention"),
            "a bare ZeroLine, not a Card wrapping it — a heading above a line \
             that says nothing is wrong would be a falsehood in its own chrome: {text}"
        );
        assert_eq!(el.query_selector_all("button").unwrap().length(), 0);
    }

    #[wasm_bindgen_test]
    fn a_healthy_queue_of_a_different_size_states_its_own_count() {
        // A second N, never 43 again: a hard-coded "43 packages" string would
        // pass the test above and only fail here, where the fixture's count
        // actually varies.
        let el = mount(|| view! { <QueueRegion packages=all_latest(7) hosts=one_signed_in() /> });
        assert!(
            el.text_content().unwrap().contains("7 packages"),
            "got: {}",
            el.text_content().unwrap()
        );
    }

    #[wasm_bindgen_test]
    fn a_healthy_queue_of_one_is_singular() {
        // `zero_line_text`'s `total == 1` branch is real code, not a case ever
        // proven by the plural fixtures above — deleting it and always taking
        // the plural arm must fail exactly here.
        let el = mount(|| view! { <QueueRegion packages=all_latest(1) hosts=one_signed_in() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("1 package"), "got: {text}");
        assert!(
            !text.contains("1 packages"),
            "singular, not the plural branch: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn action_href_names_the_right_page_and_carries_the_namespace() {
        // Swapping the Publish/Resolve arms, or returning an empty string for
        // every label, must fail here — asserted as the whole string, since a
        // substring match cannot tell a missing namespace from a present one.
        assert_eq!(
            action_href("Publish", "org/pkg"),
            "/commit?namespace=org/pkg"
        );
        assert_eq!(
            action_href("Resolve", "org/pkg"),
            "/merge?namespace=org/pkg"
        );
        assert_eq!(
            action_href("Get latest", "org/pkg"),
            "/installed-package?namespace=org/pkg&filter=unmodified"
        );
        assert_eq!(
            action_href("Choose S3 bucket", "org/pkg"),
            "/installed-package?namespace=org/pkg&filter=unmodified"
        );
        // The fifth label ruling 5 names: `[Sign in]`, which `cause_trailing`
        // builds from `sign_in_href` directly rather than through this match.
        assert_eq!(
            sign_in_href("custom.registry.io"),
            "/login?host=custom.registry.io&back=/main"
        );
    }

    #[wasm_bindgen_test]
    fn an_unknown_package_on_a_signed_in_host_has_no_button_to_press() {
        // `render(&Unknown, Site::QueueRow).action` is `None` — the honest
        // answer for a state the app has no operation to fix. Dropping
        // `tone=rendered.tone` from the `None` branch still compiles and
        // still passes every OTHER test; this is the one that must catch it.
        let el = mount(
            || view! { <QueueRegion packages=one_unknown_signed_in() hosts=one_signed_in() /> },
        );
        let text = el.text_content().unwrap();
        assert!(text.contains("Sync stopped"), "render's own words: {text}");
        assert_eq!(
            el.query_selector_all("button").unwrap().length(),
            0,
            "no cause here (signed in) and no action on this state — no button at all"
        );
    }

    #[wasm_bindgen_test]
    fn a_shared_cause_states_its_count_and_offers_the_one_fix() {
        let el =
            mount(|| view! { <QueueRegion packages=two_signed_out() hosts=one_signed_out() /> });
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Signed out from custom.registry.io"),
            "got: {text}"
        );
        assert!(text.contains("2 packages"), "got: {text}");
        assert!(
            text.contains("Sign in"),
            "host-scoped, so the cause owns the control: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn role_denied_points_at_the_fix_rather_than_duplicating_the_control() {
        // Section 5.3: a link may be duplicated across scopes, a control may not.
        // Switching role is host-scoped, so the control belongs to the Accounts
        // card and this row points at it.
        let el =
            mount(|| view! { <QueueRegion packages=one_role_denied() hosts=one_signed_in() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("No access as analyst"), "got: {text}");
        assert!(text.contains("s3://team-bucket"), "got: {text}");
        assert!(
            text.contains("Change your role in Accounts, above."),
            "the pointer line, verbatim: {text}"
        );
        assert_eq!(
            el.query_selector_all("button:not([aria-expanded])")
                .unwrap()
                .length(),
            0,
            "no [Switch role] here — that control lives in the Accounts card"
        );
    }

    #[wasm_bindgen_test]
    async fn expanding_a_cause_reveals_its_packages_and_they_do_not_repeat_the_cause() {
        // QueueRow's doc: expanding "Signed out — 11 packages" answers WHICH
        // packages, and repeating "Signed out" on all eleven is exactly the
        // redundancy the cause row exists to remove.
        let el =
            mount(|| view! { <QueueRegion packages=two_signed_out() hosts=one_signed_out() /> });
        assert!(
            !el.text_content().unwrap().contains("a/one"),
            "collapsed by default"
        );

        let expander = el.query_selector("[aria-expanded]").unwrap().unwrap();
        click(&expander);
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("a/one") && text.contains("a/two"),
            "got: {text}"
        );
        assert_eq!(
            text.matches("Signed out from").count(),
            1,
            "the cause is stated once, not once per member"
        );
    }

    #[wasm_bindgen_test]
    fn a_per_package_row_uses_the_queues_wording_not_the_lists() {
        // `render(state, Site::QueueRow)` exists precisely because two states word
        // themselves differently by site: the list says "Not the latest", the queue
        // says "Newer revision available" because it sits beside its action.
        let el = mount(|| view! { <QueueRegion packages=one_behind() hosts=one_signed_in() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("Newer revision available"), "got: {text}");
        assert!(
            !text.contains("Not the latest"),
            "that is the list's wording: {text}"
        );
    }
}

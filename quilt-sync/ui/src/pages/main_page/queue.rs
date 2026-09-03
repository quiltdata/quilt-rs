//! §4.3: the attention queue, derived. It has no payload of its own — given
//! the resolved package list and the host facts, the grouping, the counts and
//! the order are all computed here. Task 3 mounts what this module produces;
//! until then these items are read only by `cfg(test)`.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::commands::AccountHostData;
use crate::commands::MainPagePackageData;
use crate::kit::PackageState;

/// One row in the queue, in draw order: a cause shared by several packages,
/// or a package needing its own decision.
#[cfg_attr(not(test), expect(dead_code))]
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
        // Only shape-matched (`QueueItem::Package { namespace, .. }`) by this
        // task's own tests, which is a real "never read" under `cfg(test)`
        // even though the whole variant is constructed and matched — Task 3
        // reads it to render. The opposite direction from the type-level
        // attributes above: those cover the field being wholly unconstructed
        // outside `cfg(test)`, this covers it being unread inside it.
        #[cfg_attr(test, expect(dead_code))]
        state: PackageState,
    },
}

/// What a cause's trailing slot offers.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Clone, Debug)]
pub enum CauseAction {
    /// `[Sign in]`, targeting this host.
    SignIn {
        #[cfg_attr(test, expect(dead_code))]
        host: String,
    },
    /// No control here — switching role is host-scoped, so it belongs to the
    /// host row in the Accounts card; the trailing slot points at it instead
    /// of duplicating it. `None` only when the denial itself carries no host,
    /// which nothing upstream should produce but this function does not
    /// assume.
    SwitchRole {
        #[cfg_attr(test, expect(dead_code))]
        host: Option<String>,
    },
}

/// The queue, derived. Section 4.3: it has no payload — given the resolved
/// package list and the host facts, everything below is computed here.
#[cfg_attr(not(test), expect(dead_code))]
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

    for (host, members) in role_denied_groups {
        // One bucket is one denial: every member shares the same bucket by
        // construction, and the role and host it names come from the group's
        // first member in the input's own order. A difference among members
        // here would be a backend inconsistency, not a case to render twice.
        let first = members[0];
        let PackageState::RoleDenied { role } = &first.state else {
            unreachable!("role_denied_groups only ever collects RoleDenied packages")
        };
        let text = role_denied_text(role.as_deref(), first.host.as_deref(), host);
        ranked_causes.push((
            0, // §5 row 1: role-denied sorts before signed-out.
            text.clone(),
            QueueItem::Cause {
                text,
                action: CauseAction::SwitchRole {
                    host: first.host.clone(),
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AccountHostData;
    use crate::commands::MainPagePackageData;
    use crate::kit::PackageState;
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
                assert!(matches!(action, CauseAction::SignIn { .. }));
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
}

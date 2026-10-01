use std::borrow::Cow;

use quilt_rs::lineage::InstalledPackageStatus;
use quilt_rs::lineage::PackageLineage;
use quilt_rs::lineage::UpstreamState;
use quilt_uri::Namespace;
use tabled::settings::Modify;
use tabled::settings::Span;
use tracing::log;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

/// Rendered in the `bucket` cell of a package with no remote.
const NO_BUCKET: &str = "∅";

/// Printed under the table. `list` never reads the remote, so every status is
/// only as fresh as the last operation that wrote the remote tip to lineage.
const LAST_SYNCED_HINT: &str = "Statuses are as of each package's last install, pull or push. \
Run `quilt status --namespace <namespace>` to check the remote now.";

/// Printed under a `--fetch` table in place of [`LAST_SYNCED_HINT`].
const FETCHED_HINT: &str = "Statuses were checked against each package's remote just now, \
except where marked unreachable. Changed files are ones not committed yet.";

/// Appended to a `--fetch` row whose remote could not be read, so its status
/// is as of the last sync. See [`Fetched::remote_checked`].
const UNREACHABLE_SUFFIX: &str = " (remote unreachable)";

/// How many packages `--fetch` checks at once. Each check is a remote round
/// trip plus a working-tree walk, so an unbounded fan-out over a large domain
/// would open as many connections and directory scans as there are packages.
const FETCH_CONCURRENCY: usize = 8;

#[derive(Debug, Default)]
pub struct Input {
    /// Ask each package's remote for its current state, and count the files
    /// changed since the last commit, instead of reading lineage alone.
    pub fetch: bool,
}

/// A listed package as [`model`] resolves it; [`state`] words it.
pub struct PackageEntry {
    /// `None` for a local-only package — no remote, or a remote with no bucket.
    pub bucket: Option<String>,
    pub namespace: Namespace,
    pub status: UpstreamState,
    /// `Some` on every row of a `--fetch` listing, `None` on a plain one.
    pub fetched: Option<Fetched>,
}

/// What `--fetch` learned about a package beyond its [`UpstreamState`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    /// The package has a local commit. Split out because `QuiltSync` words an
    /// up-to-date package with one as "revision not published".
    pub has_local_commit: bool,
    /// Files changed since the last commit. `None` when the working tree was
    /// not scanned: no remote, a misconfigured one, or a failed status call.
    pub changed_files: Option<usize>,
    /// The remote refused the active role.
    pub access_denied: bool,
    /// The remote's `latest` tip was read in this run
    /// ([`InstalledPackageStatus::latest_refreshed`]). `false` for a package
    /// with no remote or a misconfigured one, a denial, a failed status call,
    /// and a status call that could not reach the remote and answered from the
    /// last-known tip: that last row is the only one marked unreachable.
    pub remote_checked: bool,
}

pub struct Output {
    /// Grouped by bucket. Only [`Output::new`] builds this, because
    /// [`Display`](std::fmt::Display) cannot restore the order it needs.
    installed_packages_list: Vec<PackageEntry>,
    /// Built by `--fetch`: renders the live column and hint.
    fetched: bool,
}

impl Output {
    /// Sorts each bucket's packages together, local-only ones last.
    ///
    /// `Display` spans a bucket's cell over a *run* of adjacent rows, so a
    /// bucket reached in two runs would be named twice, in two spans. Sorting
    /// here rather than trusting the caller is what makes that unreachable.
    fn new(mut installed_packages_list: Vec<PackageEntry>, fetched: bool) -> Self {
        installed_packages_list.sort_by(|a, b| {
            (a.bucket.is_none(), &a.bucket, &a.namespace).cmp(&(
                b.bucket.is_none(),
                &b.bucket,
                &b.namespace,
            ))
        });
        Self {
            installed_packages_list,
            fetched,
        }
    }

    fn column(&self) -> &'static str {
        if self.fetched {
            "status"
        } else {
            "last synced"
        }
    }

    fn hint(&self) -> &'static str {
        if self.fetched {
            FETCHED_HINT
        } else {
            LAST_SYNCED_HINT
        }
    }
}

/// Worded for `list` alone, not [`UpstreamState`]'s `Display`: the state here is
/// read from lineage, not refreshed, so each value must read as true *as of* the
/// last sync. `quilt status` refreshes the tip and words its verdict its own way.
fn last_synced(entry: &PackageEntry) -> &'static str {
    match entry.status {
        UpstreamState::UpToDate => "synced",
        UpstreamState::Ahead => "unpushed commit",
        UpstreamState::Behind => "behind",
        UpstreamState::Diverged => "diverged",
        UpstreamState::Local if entry.bucket.is_some() => "never pushed",
        UpstreamState::Local => "local only",
        UpstreamState::Error => "unknown",
    }
}

/// Worded to match `QuiltSync`'s main page (`kit::package_state::render` at
/// `Site::ListRow`, lowercased), because `--fetch` answers the question that page
/// answers. The decision is `resolve_state`'s in `QuiltSync`'s
/// `commands/main_page.rs`, mirrored arm for arm: it maps the [`UpstreamState`]
/// `quilt-rs` already derived plus the facts in [`Fetched`], and never re-derives
/// from hashes, so the two surfaces cannot disagree about a package. Copied, not
/// shared: `quilt-cli` does not depend on the app.
///
/// Two departures. `QuiltSync`'s "Sync stopped" is `unknown` here, since the CLI
/// has no sync to stop. The app-only states (a paused sync, a pull conflict)
/// come from its watcher, which the CLI does not run.
fn fetched_state(entry: &PackageEntry, fetched: &Fetched) -> Cow<'static, str> {
    if fetched.access_denied {
        return "no access".into();
    }
    // Only outranks the two states that say nothing about the remote moving.
    let pending_changes = match fetched.changed_files {
        Some(1) => Some("1 file changed".into()),
        Some(files) if files > 1 => Some(format!("{files} files changed").into()),
        _ => None,
    };
    match entry.status {
        UpstreamState::Local if entry.bucket.is_some() => "not published yet".into(),
        UpstreamState::Local => "no S3 bucket yet".into(),
        UpstreamState::Behind => "not the latest".into(),
        UpstreamState::Diverged => "changed in both places".into(),
        UpstreamState::Error => "unknown".into(),
        UpstreamState::Ahead => pending_changes.unwrap_or("revision not published".into()),
        UpstreamState::UpToDate => pending_changes.unwrap_or(if fetched.has_local_commit {
            "revision not published".into()
        } else {
            "latest".into()
        }),
    }
}

fn state(entry: &PackageEntry) -> Cow<'static, str> {
    match &entry.fetched {
        // A file count means the status call answered; without the remote, it
        // answered from the last-known tip, so the word is qualified rather
        // than passed off as fresh.
        Some(fetched) if fetched.changed_files.is_some() && !fetched.remote_checked => {
            format!("{}{UNREACHABLE_SUFFIX}", fetched_state(entry, fetched)).into()
        }
        Some(fetched) => fetched_state(entry, fetched),
        None => last_synced(entry).into(),
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }

        let mut builder = tabled::builder::Builder::default();
        builder.push_record(["bucket", "namespace", self.column()]);
        for entry in &self.installed_packages_list {
            builder.push_record([
                Cow::Borrowed(entry.bucket.as_deref().unwrap_or(NO_BUCKET)),
                Cow::Owned(entry.namespace.to_string()),
                state(entry),
            ]);
        }
        let mut table = builder.build();

        // Span each bucket's cell over its packages' rows. Not
        // `Merge::vertical()`, the built-in for this: it merges *every*
        // column, so two adjacent packages sharing a status collapse into one
        // cell and the row count stops being readable.
        //
        // `chunk_by` only sees runs, so it depends on `model` having sorted
        // equal buckets adjacent.
        let mut row = 1; // row 0 is the header
        for group in self
            .installed_packages_list
            .chunk_by(|a, b| a.bucket == b.bucket)
        {
            if group.len() > 1 {
                table.with(Modify::new((row, 0)).with(Span::row(group.len().cast_signed())));
            }
            row += group.len();
        }

        write!(f, "{table}\n{}", self.hint())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let packages: Vec<_> = self
            .installed_packages_list
            .iter()
            .map(|package| {
                let mut row = serde_json::json!({
                    "bucket": package.bucket.as_deref(),
                    "namespace": package.namespace.to_string(),
                    "status": package.status,
                });
                // Added, never changed, so a plain listing's JSON stays as it was.
                // `fetched` is whether this row's remote tip was read just now.
                if let Some(fetched) = &package.fetched {
                    row["fetched"] = fetched.remote_checked.into();
                    if let Some(files) = fetched.changed_files {
                        row["changed_files"] = files.into();
                    }
                    if fetched.access_denied {
                        row["access_denied"] = true.into();
                    }
                }
                row
            })
            .collect();
        serde_json::json!({ "packages": packages })
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.list(args).await)
}

/// The bucket cell of a row: `None` for a local-only package — no remote, or a
/// remote with no bucket.
fn bucket(lineage: &PackageLineage) -> Option<String> {
    // An empty bucket is what the cascade already reads as local-only, so it
    // renders like a package with no remote at all.
    lineage
        .remote_uri
        .as_ref()
        .map(|remote| remote.bucket.clone())
        .filter(|bucket| !bucket.is_empty())
}

/// Lists installed packages from the local domain, one read of the lineage
/// record for the whole listing.
///
/// Plain, it never touches the network: `status` is the [`UpstreamState`]
/// cascade over the lineage's four hashes, read against the *last-known* remote
/// tip. With `fetch`, each package is checked the way `QuiltSync`'s main page
/// checks it, all at once; see [`fetch_entry`].
pub async fn model(local_domain: &quilt_rs::LocalDomain, args: Input) -> Result<Output, Error> {
    let domain_lineage = local_domain.get_lineage().await?;
    let mut installed_packages_list = Vec::with_capacity(domain_lineage.packages.len());

    if !args.fetch {
        for (namespace, lineage) in domain_lineage.packages {
            installed_packages_list.push(PackageEntry {
                bucket: bucket(&lineage),
                status: UpstreamState::from(lineage),
                namespace,
                fetched: None,
            });
        }
        return Ok(Output::new(installed_packages_list, false));
    }

    // Spawned rather than awaited in turn: each package is a round trip to its
    // remote plus a walk of its working tree. At most `FETCH_CONCURRENCY` run
    // at once. The order they finish in does not matter, since `Output::new`
    // sorts.
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(FETCH_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();
    let mut spawned = std::collections::HashMap::new();
    for (namespace, lineage) in domain_lineage.packages {
        let installed_package = local_domain.create_installed_package(namespace.clone());
        let fallback = PackageEntry {
            bucket: bucket(&lineage),
            namespace: namespace.clone(),
            status: UpstreamState::Error,
            fetched: Some(Fetched {
                has_local_commit: lineage.commit.is_some(),
                ..Fetched::default()
            }),
        };
        let limit = limit.clone();
        let task = tasks.spawn(async move {
            // Held until the check ends. The semaphore is never closed, so
            // this always holds a permit.
            let _permit = limit.acquire_owned().await;
            fetch_entry(namespace, lineage, || installed_package.status(None)).await
        });
        spawned.insert(task.id(), fallback);
    }
    while let Some(joined) = tasks.join_next_with_id().await {
        match joined {
            Ok((id, entry)) => {
                spawned.remove(&id);
                installed_packages_list.push(entry);
            }
            // A task that panicked is one package's failure, like any other.
            Err(err) => {
                if let Some(fallback) = spawned.remove(&err.id()) {
                    log::warn!("Failed to check {}: {err}", fallback.namespace);
                    installed_packages_list.push(fallback);
                }
            }
        }
    }
    Ok(Output::new(installed_packages_list, true))
}

/// A bucket but no catalog host: nowhere to vend credentials from, so `QuiltSync`
/// shows `Unknown` without asking. The same predicate as its
/// `misconfigured_remote`.
fn misconfigured_remote(lineage: &PackageLineage) -> bool {
    lineage
        .remote_uri
        .as_ref()
        .is_some_and(|uri| uri.origin.is_none())
}

/// One `--fetch` row, decided as `QuiltSync`'s main page decides it.
///
/// `status` is `InstalledPackage::status`, which asks the remote for its
/// `latest` tag and scans the working tree. It is only called for a package
/// with a bucket and a catalog host: a misconfigured remote is `unknown` and a
/// package with no remote is resolved from lineage, both without the network.
/// The fetched tip is not written back to lineage, as `QuiltSync` does not either.
///
/// When the remote cannot be reached, `status` itself warns and answers from
/// the last-known tip, so that row is only as fresh as a plain listing and is
/// marked unreachable ([`Fetched::remote_checked`] stays `false`). Any
/// error it does return is logged and shows as `unknown` for that row alone,
/// except a denial, which is its own state.
async fn fetch_entry<F, Fut>(
    namespace: Namespace,
    lineage: PackageLineage,
    status: F,
) -> PackageEntry
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<InstalledPackageStatus, quilt_rs::Error>>,
{
    let bucket = bucket(&lineage);
    let mut fetched = Fetched {
        has_local_commit: lineage.commit.is_some(),
        ..Fetched::default()
    };
    let upstream = if misconfigured_remote(&lineage) {
        UpstreamState::Error
    } else if bucket.is_none() {
        UpstreamState::from(lineage)
    } else {
        match status().await {
            Ok(status) => {
                fetched.changed_files = Some(status.changes.len());
                fetched.remote_checked = status.latest_refreshed;
                status.upstream_state
            }
            Err(err) if err.is_access_denied() => {
                fetched.access_denied = true;
                UpstreamState::Error
            }
            Err(err) => {
                log::warn!("Failed to check {namespace}: {err}");
                UpstreamState::Error
            }
        }
    };
    PackageEntry {
        bucket,
        namespace,
        status: upstream,
        fetched: Some(fetched),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::Commands;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    fn entry(bucket: Option<&str>, name: &str, status: UpstreamState) -> PackageEntry {
        PackageEntry {
            bucket: bucket.map(str::to_owned),
            namespace: ("example", name).into(),
            status,
            fetched: None,
        }
    }

    fn fetched_entry(
        bucket: Option<&str>,
        name: &str,
        status: UpstreamState,
        fetched: Fetched,
    ) -> PackageEntry {
        PackageEntry {
            fetched: Some(fetched),
            ..entry(bucket, name, status)
        }
    }

    #[test(tokio::test)]
    async fn test_empty_list() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        {
            let local_domain = m.get_local_domain();
            let empty_output = model(local_domain, Input::default()).await?;
            assert!(empty_output.installed_packages_list.is_empty());
            assert_eq!(format!("{empty_output}"), "No installed packages");
        }
        Ok(())
    }

    #[test]
    fn test_json_empty_list() {
        let output = Output::new(Vec::new(), false);

        assert_eq!(output.to_json().to_string(), r#"{"packages":[]}"#);
    }

    #[test]
    fn test_json_includes_bucket_namespace_and_status() {
        let output = Output::new(
            vec![
                entry(Some("acme-research"), "one", UpstreamState::UpToDate),
                entry(None, "scratch", UpstreamState::Local),
            ],
            false,
        );

        assert_eq!(
            output.to_json().to_string(),
            r#"{"packages":[{"bucket":"acme-research","namespace":"example/one","status":"up_to_date"},{"bucket":null,"namespace":"example/scratch","status":"local"}]}"#
        );
    }

    /// Packages arrive in whatever order the lineage map yields, so the two
    /// buckets here are interleaved and the local-only package sits in the
    /// middle. Each bucket must still be named exactly once, in one span.
    #[test]
    fn test_display_groups_interleaved_buckets() {
        let output = Output::new(
            vec![
                entry(Some("zulu-bucket"), "late", UpstreamState::Behind),
                entry(Some("acme-research"), "two", UpstreamState::UpToDate),
                entry(None, "scratch", UpstreamState::Local),
                entry(Some("acme-research"), "one", UpstreamState::UpToDate),
                entry(Some("zulu-bucket"), "early", UpstreamState::Ahead),
            ],
            false,
        );

        let rendered = format!("{output}");
        assert_eq!(rendered.matches("acme-research").count(), 1, "one span");
        assert_eq!(rendered.matches("zulu-bucket").count(), 1, "one span");

        let positions = |name: &str| rendered.find(name).expect("{name} is listed");
        // Buckets sort, each bucket's own packages sort, local-only last.
        assert!(positions("acme-research") < positions("zulu-bucket"));
        assert!(positions("example/one") < positions("example/two"));
        assert!(positions("example/early") < positions("example/late"));
        assert!(positions("zulu-bucket") < positions("example/scratch"));
    }

    /// A bucket shared by several packages is named once, in a spanning cell;
    /// a bucket with one package still gets its own cell; a package with no
    /// remote names that absence.
    #[test]
    fn test_display_spans_repeated_buckets() {
        let output = Output::new(
            vec![
                entry(Some("acme-research"), "one", UpstreamState::UpToDate),
                entry(Some("acme-research"), "two", UpstreamState::UpToDate),
                entry(Some("zulu-bucket"), "late", UpstreamState::Behind),
                entry(None, "scratch", UpstreamState::Local),
            ],
            false,
        );

        let rendered = format!("{output}");
        assert_eq!(rendered.matches("acme-research").count(), 1, "named once");
        assert_eq!(rendered.matches("zulu-bucket").count(), 1);
        assert_eq!(rendered.matches(NO_BUCKET).count(), 1);
        for name in [
            "example/one",
            "example/two",
            "example/late",
            "example/scratch",
        ] {
            assert!(rendered.contains(name), "{name} is listed");
        }
        // One header, so `last synced` names the column and nothing else.
        assert_eq!(rendered.matches("last synced").count(), 1);
        // Statuses are never merged, even when adjacent rows repeat one.
        assert_eq!(rendered.matches("| synced").count(), 2);
    }

    /// Each state reads as true *as of* the last sync, and a local-only package
    /// says whether it has a remote it was never pushed to.
    #[test]
    fn test_display_words_each_state_as_of_last_sync() {
        let cases = [
            (Some("acme-research"), UpstreamState::UpToDate, "synced"),
            (
                Some("acme-research"),
                UpstreamState::Ahead,
                "unpushed commit",
            ),
            (Some("acme-research"), UpstreamState::Behind, "behind"),
            (Some("acme-research"), UpstreamState::Diverged, "diverged"),
            (Some("acme-research"), UpstreamState::Local, "never pushed"),
            (None, UpstreamState::Local, "local only"),
            (Some("acme-research"), UpstreamState::Error, "unknown"),
        ];
        for (bucket, status, expected) in cases {
            assert_eq!(last_synced(&entry(bucket, "one", status)), expected);
        }
    }

    /// The hint follows the table, so a reader learns how stale it may be.
    /// An empty listing has no statuses to qualify, so it prints no hint.
    #[test]
    fn test_display_ends_with_last_synced_hint() {
        let output = Output::new(
            vec![entry(Some("acme-research"), "one", UpstreamState::UpToDate)],
            false,
        );
        assert!(format!("{output}").ends_with(LAST_SYNCED_HINT));

        let empty = Output::new(Vec::new(), false);
        assert!(!format!("{empty}").contains(LAST_SYNCED_HINT));
    }

    /// A row as `fetch_entry` builds it when the status call reaches the
    /// remote: a file count comes with a checked tip, no count with none.
    fn fetched(changed_files: Option<usize>) -> Fetched {
        Fetched {
            changed_files,
            remote_checked: changed_files.is_some(),
            ..Fetched::default()
        }
    }

    /// A row whose status call answered from the last-known tip.
    fn unreachable(changed_files: usize) -> Fetched {
        Fetched {
            remote_checked: false,
            ..fetched(Some(changed_files))
        }
    }

    /// Each state reads as `QuiltSync`'s main page words it, lowercased, and is
    /// decided the way its `resolve_state` decides it: a denial outranks
    /// everything, a file count outranks only the two states that say nothing
    /// about the remote having moved.
    #[test]
    fn test_display_words_each_fetched_state_as_quilt_sync_does() {
        let bucket = Some("acme-research");
        let committed = Fetched {
            has_local_commit: true,
            ..fetched(Some(0))
        };
        let denied = Fetched {
            access_denied: true,
            ..fetched(None)
        };
        let cases = [
            (bucket, UpstreamState::UpToDate, fetched(Some(0)), "latest"),
            (bucket, UpstreamState::UpToDate, fetched(None), "latest"),
            (
                bucket,
                UpstreamState::UpToDate,
                committed.clone(),
                "revision not published",
            ),
            (
                bucket,
                UpstreamState::UpToDate,
                fetched(Some(1)),
                "1 file changed",
            ),
            (
                bucket,
                UpstreamState::UpToDate,
                Fetched {
                    changed_files: Some(3),
                    ..committed.clone()
                },
                "3 files changed",
            ),
            (
                bucket,
                UpstreamState::Ahead,
                fetched(Some(0)),
                "revision not published",
            ),
            (
                bucket,
                UpstreamState::Ahead,
                fetched(Some(2)),
                "2 files changed",
            ),
            (
                bucket,
                UpstreamState::Behind,
                fetched(Some(0)),
                "not the latest",
            ),
            (
                bucket,
                UpstreamState::Behind,
                fetched(Some(2)),
                "not the latest",
            ),
            (
                bucket,
                UpstreamState::Diverged,
                fetched(Some(2)),
                "changed in both places",
            ),
            (
                bucket,
                UpstreamState::Local,
                fetched(Some(2)),
                "not published yet",
            ),
            (
                None,
                UpstreamState::Local,
                fetched(None),
                "no S3 bucket yet",
            ),
            (bucket, UpstreamState::Error, fetched(None), "unknown"),
            (bucket, UpstreamState::Error, denied, "no access"),
        ];
        for (bucket, status, fetched, expected) in cases {
            let entry = fetched_entry(bucket, "one", status, fetched);
            assert_eq!(state(&entry), expected, "{status:?} {:?}", entry.fetched);
        }
    }

    /// A status call that could not reach the remote keeps its word but says
    /// so. Rows that never asked the remote, or got no answer, are not marked:
    /// their words already say what is known.
    #[test]
    fn test_display_marks_rows_whose_remote_was_unreachable() {
        let bucket = Some("acme-research");
        let cases = [
            (
                bucket,
                UpstreamState::UpToDate,
                unreachable(0),
                "latest (remote unreachable)",
            ),
            (
                bucket,
                UpstreamState::UpToDate,
                unreachable(1),
                "1 file changed (remote unreachable)",
            ),
            (
                bucket,
                UpstreamState::Behind,
                unreachable(0),
                "not the latest (remote unreachable)",
            ),
            (
                None,
                UpstreamState::Local,
                fetched(None),
                "no S3 bucket yet",
            ),
            (bucket, UpstreamState::Error, fetched(None), "unknown"),
            (
                bucket,
                UpstreamState::Error,
                Fetched {
                    access_denied: true,
                    ..fetched(None)
                },
                "no access",
            ),
        ];
        for (bucket, status, fetched, expected) in cases {
            let entry = fetched_entry(bucket, "one", status, fetched);
            assert_eq!(state(&entry), expected, "{status:?} {:?}", entry.fetched);
        }
    }

    /// The column and the hint say which question the table answers.
    #[test]
    fn test_display_names_the_fetched_column_and_hint() {
        let output = Output::new(
            vec![fetched_entry(
                Some("acme-research"),
                "one",
                UpstreamState::UpToDate,
                fetched(Some(0)),
            )],
            true,
        );
        let rendered = format!("{output}");
        assert!(rendered.ends_with(FETCHED_HINT));
        assert!(!rendered.contains(LAST_SYNCED_HINT));
        assert!(!rendered.contains("last synced"));
        assert!(rendered.contains("| status"));
        assert!(rendered.contains("| latest"));

        let empty = Output::new(Vec::new(), true);
        assert_eq!(format!("{empty}"), "No installed packages");
    }

    /// Fetch mode only adds keys; `status` keeps its values, and a key with
    /// nothing to say is left out rather than sent as null. `fetched` is on
    /// every row and is `true` only where the remote tip was read just now.
    #[test]
    fn test_json_adds_fetched_fields() {
        let output = Output::new(
            vec![
                fetched_entry(
                    Some("acme-research"),
                    "one",
                    UpstreamState::UpToDate,
                    fetched(Some(2)),
                ),
                fetched_entry(
                    Some("acme-research"),
                    "two",
                    UpstreamState::Error,
                    Fetched {
                        access_denied: true,
                        ..fetched(None)
                    },
                ),
                fetched_entry(
                    Some("acme-research"),
                    "three",
                    UpstreamState::UpToDate,
                    unreachable(1),
                ),
                fetched_entry(None, "scratch", UpstreamState::Local, fetched(None)),
            ],
            true,
        );

        assert_eq!(
            output.to_json().to_string(),
            r#"{"packages":[{"bucket":"acme-research","namespace":"example/one","status":"up_to_date","fetched":true,"changed_files":2},{"bucket":"acme-research","namespace":"example/three","status":"up_to_date","fetched":false,"changed_files":1},{"bucket":"acme-research","namespace":"example/two","status":"error","fetched":false,"access_denied":true},{"bucket":null,"namespace":"example/scratch","status":"local","fetched":false}]}"#
        );
    }

    fn remote_lineage(origin: Option<&str>) -> PackageLineage {
        PackageLineage {
            remote_uri: Some(quilt_uri::ManifestUri {
                origin: origin.map(|host| host.parse().expect("a valid host")),
                bucket: "acme-research".to_string(),
                namespace: ("example", "one").into(),
                hash: "abc".to_string(),
            }),
            base_hash: "abc".to_string(),
            latest_hash: "abc".to_string(),
            ..PackageLineage::default()
        }
    }

    fn s3_error(kind: quilt_rs::S3ErrorKind) -> quilt_rs::Error {
        quilt_rs::Error::S3(quilt_rs::S3Error::new(kind))
    }

    /// The status call's answer is the row's: its state, its file count, and
    /// whether it reached the remote.
    #[test(tokio::test)]
    async fn test_fetch_entry_takes_the_status_calls_answer() {
        let row = |key: &str| quilt_rs::manifest::ManifestRow {
            logical_key: key.into(),
            physical_key: String::new(),
            hash: quilt_rs::object_hash::ObjectHash::default(),
            size: 0,
            meta: None,
        };
        let changes = std::collections::BTreeMap::from([
            (
                "a.csv".into(),
                quilt_rs::lineage::Change::Added(row("a.csv")),
            ),
            (
                "b.csv".into(),
                quilt_rs::lineage::Change::Removed(row("b.csv")),
            ),
        ]);
        let entry = fetch_entry(
            ("example", "one").into(),
            remote_lineage(Some("example.com")),
            || async {
                Ok(InstalledPackageStatus {
                    latest_refreshed: true,
                    ..InstalledPackageStatus::new(UpstreamState::Behind, changes)
                })
            },
        )
        .await;

        assert_eq!(entry.bucket.as_deref(), Some("acme-research"));
        assert_eq!(entry.status, UpstreamState::Behind);
        assert_eq!(entry.fetched, Some(fetched(Some(2))));
        assert_eq!(state(&entry), "not the latest");

        let stale = fetch_entry(
            ("example", "one").into(),
            remote_lineage(Some("example.com")),
            || async {
                Ok(InstalledPackageStatus::new(
                    UpstreamState::UpToDate,
                    quilt_rs::lineage::ChangeSet::new(),
                ))
            },
        )
        .await;
        assert_eq!(stale.fetched, Some(unreachable(0)));
        assert_eq!(state(&stale), "latest (remote unreachable)");
    }

    /// A denial is a state of its own; any other failure is `unknown` for that
    /// row, not an error for the listing.
    #[test(tokio::test)]
    async fn test_fetch_entry_falls_back_per_row_on_error() {
        let denied = fetch_entry(
            ("example", "one").into(),
            remote_lineage(Some("example.com")),
            || async {
                Err(s3_error(quilt_rs::S3ErrorKind::AccessDenied(
                    "denied".to_string(),
                )))
            },
        )
        .await;
        assert_eq!(denied.status, UpstreamState::Error);
        assert!(denied.fetched.as_ref().is_some_and(|f| f.access_denied));
        assert_eq!(state(&denied), "no access");

        for err in [
            s3_error(quilt_rs::S3ErrorKind::GetObject("boom".to_string())),
            quilt_rs::Error::Login(quilt_rs::LoginError::NoSession(None)),
        ] {
            let failed = fetch_entry(
                ("example", "one").into(),
                remote_lineage(Some("example.com")),
                || async { Err(err) },
            )
            .await;
            assert_eq!(failed.status, UpstreamState::Error);
            assert_eq!(failed.fetched, Some(fetched(None)));
            assert_eq!(state(&failed), "unknown");
        }
    }

    /// `QuiltSync`'s two short-circuits: a bucket with no catalog host is
    /// `unknown` and a package with no remote is read from lineage, neither
    /// asking the remote.
    #[test(tokio::test)]
    async fn test_fetch_entry_skips_the_remote_when_quilt_sync_does() {
        let never = || async { panic!("the remote must not be asked") };

        let misconfigured =
            fetch_entry(("example", "one").into(), remote_lineage(None), never).await;
        assert_eq!(misconfigured.status, UpstreamState::Error);
        assert_eq!(state(&misconfigured), "unknown");

        let local = fetch_entry(
            ("example", "scratch").into(),
            PackageLineage::default(),
            never,
        )
        .await;
        assert_eq!(local.bucket, None);
        assert_eq!(local.status, UpstreamState::Local);
        assert_eq!(local.fetched, Some(fetched(None)));
        assert_eq!(state(&local), "no S3 bucket yet");
    }

    /// End to end over a real domain, which needs no network for a package
    /// with no remote.
    #[test(tokio::test)]
    async fn test_model_fetch_with_local_package() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        m.create(crate::cli::create::Input {
            namespace: ("example", "local").into(),
            source: None,
            message: None,
        })
        .await?;

        let output = m.list(Input { fetch: true }).await?;
        assert_eq!(output.installed_packages_list.len(), 1);
        let rendered = format!("{output}");
        assert!(rendered.contains("| no S3 bucket yet"));
        assert!(rendered.ends_with(FETCHED_HINT));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_model_with_local_package() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        m.create(crate::cli::create::Input {
            namespace: ("example", "local").into(),
            source: None,
            message: None,
        })
        .await?;

        let output = model(m.get_local_domain(), Input::default()).await?;
        let entry = &output.installed_packages_list[0];
        assert_eq!(entry.bucket, None);
        assert_eq!(entry.namespace, ("example", "local").into());
        assert_eq!(entry.status, UpstreamState::Local);

        let output = format!("{output}");
        assert!(output.contains("example/local"));
        assert!(output.contains(NO_BUCKET));

        Ok(())
    }

    /// Verifies that list model returns correct output for both empty and populated states:
    ///   * empty list shows "No installed packages" message
    ///   * after installing a package, shows its bucket, namespace and status
    ///
    /// This uses the shared S3 fixture and is run by credentialed CI only.
    #[test(tokio::test)]
    async fn live_model() -> Result<(), Error> {
        // Test with one installed package
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;
        {
            let local_domain = m.get_local_domain();
            let output = model(local_domain, Input::default()).await?;

            let entry = &output.installed_packages_list[0];
            assert_eq!(entry.bucket.as_deref(), Some(pkg::BUCKET));
            assert_eq!(entry.namespace, pkg::NAMESPACE.into());
            assert_eq!(entry.status, UpstreamState::UpToDate);

            let output = format!("{output}");
            assert!(output.contains("bucket"));
            assert!(output.contains("namespace"));
            assert!(output.contains("last synced"));
            assert!(output.contains(pkg::BUCKET));
            assert!(output.contains("| synced"));
        }

        Ok(())
    }

    /// Verifies that list command returns correct output after installing a package:
    ///   * shows the installed package namespace
    ///   * formats output according to display implementation
    // TODO: install and list multiple packages
    ///
    /// This uses the shared S3 fixture and is run by credentialed CI only.
    #[test(tokio::test)]
    async fn live_command_with_package() -> Result<(), Error> {
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;

        if let Std::Out(output) = command(m, Input::default()).await {
            let output = output.to_string();
            assert!(output.contains(pkg::BUCKET));
            assert!(output.contains(pkg::NAMESPACE_STR));
            assert!(output.contains("| synced"));
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}

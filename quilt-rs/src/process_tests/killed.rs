//! Validation items 2 and 3 of
//! <https://github.com/quiltdata/quilt-specs/pull/292>: each operation killed
//! at each failpoint it passes, on each store, then run again as a user
//! would, and the state it leaves classified.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::*;
use crate::io::manifest::resolve_tag;
use crate::io::storage::Storage;
use quilt_uri::Tag;

/// Where a failpoint sits, named as the spec names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    /// Between the last working-tree change and the lineage write.
    BeforeLineage,
    BeforeTemp,
    AfterTemp,
    /// After the rename, before the folder is synced.
    AfterRename,
    /// After a new owner folder is created, before its parent is synced.
    AfterOwnerDir,
    /// After a ref is unlinked, before its folder is synced.
    AfterUnlink,
}

impl Step {
    const ALL: [Step; 6] = [
        Step::BeforeLineage,
        Step::BeforeTemp,
        Step::AfterTemp,
        Step::AfterRename,
        Step::AfterOwnerDir,
        Step::AfterUnlink,
    ];

    fn describe(self) -> &'static str {
        match self {
            Step::BeforeLineage => "after the working tree, before the lineage write",
            Step::BeforeTemp => "before the temp file is written",
            Step::AfterTemp => "after the temp file, before the rename",
            Step::AfterRename => "after the rename, before the folder sync",
            Step::AfterOwnerDir => "after a new owner folder, before its parent's sync",
            Step::AfterUnlink => "after the ref's unlink, before its folder's sync",
        }
    }

    /// The failpoint `operation` passes at this step on `store`, if any.
    fn failpoint(self, operation: Operation, store: LineageStore) -> Option<String> {
        let tag = match store {
            LineageStore::DataJson => "data.json",
            LineageStore::Refs => "ref",
        };
        // Removing a ref unlinks it, and writes no ref.
        let writes = !(operation == Operation::Uninstall && store == LineageStore::Refs);
        match self {
            Step::BeforeLineage => match operation {
                Operation::Install => Some("flow.install.before-create".to_string()),
                Operation::Uninstall => None,
                _ => Some(format!("flow.{}.before-lineage", operation.name())),
            },
            Step::BeforeTemp => writes.then(|| format!("{tag}.before-temp")),
            Step::AfterTemp => writes.then(|| format!("{tag}.after-temp")),
            Step::AfterRename => writes.then(|| format!("{tag}.after-rename")),
            Step::AfterOwnerDir => (operation == Operation::Install && store == LineageStore::Refs)
                .then(|| "ref.create.after-owner-dir".to_string()),
            Step::AfterUnlink => (operation == Operation::Uninstall && store == LineageStore::Refs)
                .then(|| "ref.remove.after-unlink".to_string()),
        }
    }
}

/// The outcomes the spec records for a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// The operation run again leaves the state an uninterrupted run leaves.
    Recovered,
    /// It does not, and nothing the user made is gone.
    ManualFix,
    /// Something the user made is gone.
    Lost,
}

impl Outcome {
    fn describe(self) -> &'static str {
        match self {
            Outcome::Recovered => "recovered unaided",
            Outcome::ManualFix => "needed a manual fix",
            Outcome::Lost => "lost data",
        }
    }
}

/// What one kill left and what running the operation again made of it.
#[derive(Debug)]
struct Cell {
    outcome: Outcome,
    /// The target's entry could not be read after the kill.
    unreadable: bool,
    /// Temp files left in `.quilt` after the sweep.
    temps: usize,
    notes: Vec<String>,
}

impl World {
    /// The target as each operation starts from: published at a first
    /// revision, then installed with its files (except for an install),
    /// then edited (commit), edited and committed (push), or behind a
    /// second revision (pull). Returns the argument the operation takes.
    async fn set_up(&self, operation: Operation) -> Res<String> {
        let rev1 = self
            .publish(&[("a.txt", b"one"), ("b.txt", b"same")])
            .await?;
        if operation == Operation::Install {
            return Ok(rev1.hash);
        }
        self.domain().install_package(&rev1).await?;
        self.package()
            .install_paths(&["a.txt".into(), "b.txt".into()])
            .await?;
        match operation {
            Operation::Commit => std::fs::write(self.working_file("a.txt"), b"edit")?,
            Operation::Push => {
                std::fs::write(self.working_file("a.txt"), b"edit")?;
                Operation::Commit.run(self, "").await?;
                // The mock remote hashes an upload's source from its own
                // storage, so each committed object is copied there.
                let remote = self.remote();
                for row in self.package().manifest().await?.rows {
                    let Ok(object) = url::Url::parse(&row.physical_key)?.to_file_path() else {
                        continue;
                    };
                    remote
                        .storage
                        .write_byte_stream(&object, ByteStream::from(std::fs::read(&object)?))
                        .await?;
                }
            }
            Operation::Pull => {
                self.publish(&[("a.txt", b"two"), ("b.txt", b"same")])
                    .await?;
            }
            _ => {}
        }
        Ok(String::new())
    }

    fn file(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.working_file(key)).ok()
    }

    /// Whether the target is as an uninterrupted `operation` leaves it, and
    /// whether anything the user made is gone.
    async fn judge(&self, operation: Operation) -> Res<(bool, bool)> {
        let latest = resolve_tag(&self.remote(), None, &revision(String::new()), Tag::Latest)
            .await?
            .hash;
        let has = |key: &str, body: &[u8]| self.file(key).as_deref() == Some(body);
        let edited = has("a.txt", b"edit") && has("b.txt", b"same");
        let lost = match operation {
            Operation::Commit | Operation::Push => !edited,
            Operation::Pull => !has("b.txt", b"same"),
            _ => false,
        };
        let entry = self.domain().read_packages().await?.remove(&target());
        let Some(lineage) = entry.and_then(Result::ok) else {
            // Uninstalling a package that is not installed creates its
            // folder, empty, killed or not.
            let mut folder = std::fs::read_dir(self.working_file(""))
                .into_iter()
                .flatten();
            return Ok((
                operation == Operation::Uninstall && folder.next().is_none(),
                lost,
            ));
        };
        let package = self.package();
        // Every revision in this fixture holds both files.
        let whole = package.manifest().await.is_ok_and(|manifest| {
            let rows: Vec<_> = manifest
                .rows
                .iter()
                .map(|row| row.logical_key.to_str())
                .collect();
            rows == [Some("a.txt"), Some("b.txt")]
        });
        let clean = package
            .status(Some(HostConfig::default()))
            .await
            .is_ok_and(|status| status.changes.is_empty());
        let synced = lineage.remote_uri.as_ref().map(|uri| &uri.hash) == Some(&lineage.base_hash)
            && lineage.base_hash == latest;
        let done = whole
            && clean
            && match operation {
                Operation::Commit => lineage.commit.is_some() && edited,
                Operation::Push => lineage.commit.is_none() && synced && edited,
                Operation::Pull => synced && has("a.txt", b"two") && has("b.txt", b"same"),
                Operation::Install => synced,
                Operation::Uninstall => false,
            };
        Ok((done, lost))
    }

    /// The temp files left anywhere in the domain's `.quilt`.
    fn temps(&self) -> usize {
        fn count(dir: &Path) -> usize {
            std::fs::read_dir(dir).map_or(0, |entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        let path = entry.path();
                        if path.is_dir() {
                            count(&path)
                        } else {
                            usize::from(entry.file_name().to_string_lossy().contains(".tmp-"))
                        }
                    })
                    .sum()
            })
        }
        count(&self.root.join("domain").join(".quilt"))
    }
}

/// Sets `operation` up on `store`, runs it in a child killed at `failpoint`
/// (or not killed, for `None`), then sweeps and runs it again, as the CLI's
/// next start and a user's retry do.
async fn run(operation: Operation, store: LineageStore, failpoint: Option<&str>) -> Res<Cell> {
    let dir = TempDir::new()?;
    let world = World::new(&dir, store).await?;
    let arg = world.set_up(operation).await?;
    let bystander_before = world.domain().read_packages().await?.remove(&bystander());

    let mut notes = Vec::new();
    if let Some(name) = failpoint {
        assert!(
            killed_at(&world, operation, &arg, name)?,
            "{} on {store:?} never reached {name}",
            operation.name()
        );
    } else {
        let mut child = spawn(&world, operation, &arg)?;
        let deadline = Instant::now() + LIMIT;
        while child.try_wait()?.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
        let status = child.wait()?;
        assert!(
            status.success(),
            "{} on {store:?}: {status}",
            operation.name()
        );
    }

    let domain = world.domain();
    let entry = domain.read_packages().await?.remove(&target());
    let unreadable = entry.is_some_and(|entry| entry.is_err());
    domain.sweep_leftovers().await?;
    let mut timed_out = false;
    if failpoint.is_some() {
        match tokio::time::timeout(LIMIT, operation.run(&world, &arg)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => notes.push(format!("run again: {err}")),
            Err(_) => {
                timed_out = true;
                notes.push("run again: timed out".to_string());
            }
        }
    }
    let (done, lost) = world.judge(operation).await?;
    let done = done && !timed_out;
    let bystander_after = domain.read_packages().await?.remove(&bystander());
    let bystander_kept = matches!(
        (&bystander_before, &bystander_after),
        (Some(Ok(before)), Some(Ok(after))) if before == after
    );
    if !bystander_kept {
        notes.push("another package changed".to_string());
    }
    let temps = world.temps();
    if temps > 0 {
        notes.push(format!("{temps} temp file(s) left"));
    }
    let outcome = if lost || !bystander_kept {
        Outcome::Lost
    } else if done {
        Outcome::Recovered
    } else {
        Outcome::ManualFix
    };
    Ok(Cell {
        outcome,
        unreadable,
        temps,
        notes,
    })
}

/// How long an operation may take before its cell fails.
const LIMIT: Duration = Duration::from_secs(30);

const OPERATIONS: [Operation; 5] = [
    Operation::Commit,
    Operation::Pull,
    Operation::Install,
    Operation::Push,
    Operation::Uninstall,
];

const STORES: [LineageStore; 2] = [LineageStore::DataJson, LineageStore::Refs];

type Matrix = BTreeMap<(usize, Step, usize), Cell>;

/// The matrix as Markdown: a row per operation and failpoint, a column per
/// store, `n/a` where the operation passes no such failpoint on that store.
fn render(matrix: &Matrix) -> String {
    let mut out = String::from("| Operation | Killed | data.json | refs |\n|---|---|---|---|\n");
    for (o, operation) in OPERATIONS.iter().enumerate() {
        for step in Step::ALL {
            let cells: Vec<String> = (0..STORES.len())
                .map(|s| match matrix.get(&(o, step, s)) {
                    None => "n/a".to_string(),
                    Some(cell) if cell.notes.is_empty() => cell.outcome.describe().to_string(),
                    Some(cell) => {
                        format!("{} ({})", cell.outcome.describe(), cell.notes.join("; "))
                    }
                })
                .collect();
            if cells.iter().all(|cell| cell == "n/a") {
                continue;
            }
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                operation.name(),
                step.describe(),
                cells.join(" | ")
            );
        }
    }
    out
}

/// Prints the matrix, to be pasted into the spec: run it with
/// `--no-capture`.
#[test]
fn every_kill_recovers_unaided() -> Res {
    let mut jobs = Vec::new();
    for (o, &operation) in OPERATIONS.iter().enumerate() {
        for (s, &store) in STORES.iter().enumerate() {
            for step in Step::ALL {
                if let Some(name) = step.failpoint(operation, store) {
                    jobs.push(((o, step, s), operation, store, Some(name)));
                }
            }
            jobs.push(((o, Step::BeforeLineage, s), operation, store, None));
        }
    }
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(key, operation, store, name)| {
                scope.spawn(move || {
                    let cell = runtime().expect("a runtime").block_on(run(
                        *operation,
                        *store,
                        name.as_deref(),
                    ));
                    (*key, name.is_some(), cell)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("a cell"))
            .collect()
    });
    let mut matrix = Matrix::new();
    for (key, killed, cell) in results {
        let cell = cell?;
        if killed {
            matrix.insert(key, cell);
        } else {
            assert_eq!(
                (cell.outcome, cell.notes.as_slice()),
                (Outcome::Recovered, [].as_slice()),
                "an uninterrupted run is judged done: {key:?}"
            );
        }
    }
    let rendered = render(&matrix);
    println!("{rendered}");
    for ((o, step, s), cell) in &matrix {
        let at = (OPERATIONS[*o].name(), step, STORES[*s]);
        assert!(!cell.unreadable, "a kill left an unreadable entry: {at:?}");
        assert_eq!(cell.outcome, Outcome::Recovered, "{at:?}\n{rendered}");
        if STORES[*s] == LineageStore::Refs {
            assert_eq!(
                cell.temps, 0,
                "the sweep leaves no temp: {at:?}\n{rendered}"
            );
        }
    }
    Ok(())
}

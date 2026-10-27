use super::*;

use std::collections::BTreeMap;

use quilt_uri::fixtures;
use serde_json::json;
use tempfile::TempDir;

use crate::model::MockQuiltModel;

fn namespace() -> Namespace {
    ("acme", "demo").into()
}

fn settings() -> TaskSettings {
    TaskSettings {
        blessed_buckets: vec!["s3://quilt-dev".to_string()],
        agents: BTreeMap::from([(
            "kiro".to_string(),
            AgentSettings {
                command: vec!["kiro-cli-chat".to_string(), "acp".to_string()],
                trust_all: true,
            },
        )]),
    }
}

fn unclaimed() -> Value {
    json!({
        "title": "kept",
        "agent_exec": {
            "document": "TASK.md",
            "agent": { "id": "kiro" },
            "claim": { "status": "unclaimed" },
        },
    })
}

fn without_claim(meta: &Value) -> Value {
    let mut meta = meta.clone();
    meta["agent_exec"]
        .as_object_mut()
        .expect("agent_exec")
        .remove("claim");
    meta
}

/// A revision a check published.
struct Revision {
    meta: Value,
    message: String,
    /// The invocation IDs this desktop had on disk when it was pushed.
    own_at_push: BTreeSet<String>,
    /// The changed paths it committed.
    paths: BTreeSet<String>,
}

type Published = Arc<Mutex<Vec<Revision>>>;

fn changed_paths(status: Option<&quilt::lineage::InstalledPackageStatus>) -> BTreeSet<String> {
    status
        .map(|s| s.changes.keys().map(|p| p.display().to_string()).collect())
        .unwrap_or_default()
}

/// The agent's `out.md`, left in the tree.
fn written() -> BTreeMap<PathBuf, quilt::lineage::Change> {
    BTreeMap::from([(
        PathBuf::from("out.md"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    )])
}

/// An up-to-date tree with `changes`, read against a `latest` fetched just now.
fn fresh(
    changes: BTreeMap<PathBuf, quilt::lineage::Change>,
) -> quilt::lineage::InstalledPackageStatus {
    quilt::lineage::InstalledPackageStatus {
        latest_refreshed: true,
        latest_hash: "h0".to_string(),
        ..quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes)
    }
}

/// Record each push of a pending commit, certified as `certified_latest`, and
/// each reset to `latest`, in `events`.
fn expect_push_and_reset(
    model: &mut MockQuiltModel,
    events: &Arc<Mutex<Vec<String>>>,
    certified_latest: bool,
) {
    let pushed = Arc::clone(events);
    model.expect_locked_package_push().returning(move |_| {
        pushed.lock().unwrap().push("push".to_string());
        Ok(quilt::PushOutcome {
            manifest_uri: quilt_uri::ManifestUri {
                bucket: "quilt-dev".to_string(),
                namespace: namespace(),
                hash: "c1".to_string(),
                origin: Some(fixtures::host()),
            },
            certified_latest,
        })
    });
    let reset = Arc::clone(events);
    model
        .expect_locked_package_reset_local()
        .returning(move |_| {
            reset.lock().unwrap().push("reset".to_string());
            Ok(quilt_uri::ManifestUri {
                bucket: "quilt-dev".to_string(),
                namespace: namespace(),
                hash: "h2".to_string(),
                origin: Some(fixtures::host()),
            })
        });
}

/// Record each download in `events`, refusing, as the engine does, to write
/// over anything already in `home`, and skipping the `gone` paths, whose bytes
/// the remote no longer holds.
fn expect_downloads(
    model: &mut MockQuiltModel,
    events: &Arc<Mutex<Vec<String>>>,
    home: PathBuf,
    gone: &'static [&'static str],
) {
    let events = Arc::clone(events);
    model
        .expect_locked_package_install_paths()
        .returning(move |_, paths| {
            let mut report = quilt::flow::InstallPathsReport::default();
            for path in paths {
                events
                    .lock()
                    .unwrap()
                    .push(format!("download {}", path.display()));
                if std::fs::symlink_metadata(home.join(path)).is_ok() {
                    return Err(Error::General(format!("{} exists", path.display())));
                }
                if gone.iter().any(|gone| path == std::path::Path::new(gone)) {
                    report.skipped.push(path.clone());
                }
            }
            Ok(report)
        });
}

/// The remote's `latest` carries `meta` and lists `paths`.
fn expect_latest(model: &mut MockQuiltModel, meta: Value, paths: &'static [&'static str]) {
    model
        .expect_browse_remote_manifest()
        .returning(move |_| Ok(manifest(&meta, paths)));
}

fn manifest(meta: &Value, paths: &[&str]) -> quilt::manifest::Manifest {
    quilt::manifest::Manifest {
        header: quilt::manifest::ManifestHeader {
            user_meta: Some(meta.clone()),
            ..quilt::manifest::ManifestHeader::default()
        },
        rows: paths
            .iter()
            .map(|key| quilt::manifest::ManifestRow {
                logical_key: PathBuf::from(key),
                ..quilt::manifest::ManifestRow::default()
            })
            .collect(),
    }
}

/// Tasks in a temporary data folder that keep every run they are asked to
/// start, every revision they publish, and, in order, every download, walk,
/// publish, push and reset.
struct Fixture {
    dir: TempDir,
    published: Published,
    events: Arc<Mutex<Vec<String>>>,
    runs: Arc<Mutex<Vec<Run>>>,
    tasks: Tasks,
    /// The package has a commit not yet pushed.
    pending_commit: bool,
    /// The pending commit sits on another one.
    stacked_commits: bool,
    /// What a revision later still, `h3`, carries when the re-read's pull lands it.
    pulled_meta: Option<Value>,
    /// What the remote's `latest` carries; the head's metadata unless set.
    latest_meta: Option<Value>,
    /// What the head a run ends on carries; [`edited_head`] unless set.
    finished_meta: Option<Value>,
    /// Walking the tree the run ended on fails, as it does while signed out.
    walk_fails: bool,
    /// The paths this copy already tracks.
    tracked: &'static [&'static str],
    /// The paths the head lists.
    rows: &'static [&'static str],
    /// The paths whose bytes the remote no longer holds.
    gone: &'static [&'static str],
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let runs: Arc<Mutex<Vec<Run>>> = Arc::default();
        let recording = Arc::clone(&runs);
        let tasks = Tasks::new(
            dir.path().to_path_buf(),
            Arc::new(move |run| recording.lock().unwrap().push(run)),
        );
        Self {
            dir,
            published: Published::default(),
            events: Arc::default(),
            runs,
            tasks,
            pending_commit: false,
            stacked_commits: false,
            pulled_meta: None,
            latest_meta: None,
            finished_meta: None,
            walk_fails: false,
            tracked: &[],
            rows: &["TASK.md", "sub/TASK.md"],
            gone: &[],
        }
    }

    /// The package's lineage: from `quilt-dev`, at `h0`.
    fn lineage(&self) -> quilt::lineage::PackageLineage {
        let remote = quilt_uri::ManifestUri {
            bucket: "quilt-dev".to_string(),
            namespace: namespace(),
            hash: "h0".to_string(),
            origin: Some(fixtures::host()),
        };
        let mut lineage = quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string());
        for path in self.tracked {
            lineage
                .paths
                .insert(PathBuf::from(path), quilt::lineage::PathState::default());
        }
        if self.pending_commit {
            let prev_hashes = if self.stacked_commits {
                vec!["c0".to_string()]
            } else {
                Vec::new()
            };
            lineage.commit = Some(quilt::lineage::CommitState {
                timestamp: chrono::Utc::now(),
                hash: "c1".to_string(),
                prev_hashes,
            });
        }
        lineage
    }

    /// The package's folder, empty unless a test puts files in it.
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn metas(&self) -> Vec<Value> {
        let published = self.published.lock().unwrap();
        published
            .iter()
            .map(|revision| revision.meta.clone())
            .collect()
    }

    /// Check a package in a blessed bucket whose head carries `meta` and lists
    /// the fixture's `rows`, whose tree walks as `status`, publishing with
    /// `certified_latest`.
    async fn check(
        &self,
        meta: Value,
        status: quilt::lineage::InstalledPackageStatus,
        certified_latest: bool,
    ) {
        let model = self.model(meta, status, certified_latest);
        let pkg = quilt::LocalDomain::new(PathBuf::new()).create_installed_package(namespace());
        check(&model, &self.tasks, &settings(), &pkg).await;
    }

    fn model(
        &self,
        meta: Value,
        status: quilt::lineage::InstalledPackageStatus,
        certified_latest: bool,
    ) -> MockQuiltModel {
        let lineage = self.lineage();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| Ok(lineage.clone()));
        expect_push_and_reset(&mut model, &self.events, certified_latest);
        let latest = self.latest_meta.clone().unwrap_or(meta.clone());
        expect_latest(&mut model, latest, &[]);
        let rows = self.rows;
        model
            .expect_get_installed_package_manifest()
            .returning(move |_, _| Ok(manifest(&meta, rows)));
        model
            .expect_try_lock_package()
            .returning(|p| Ok(Some(p.namespace.clone())));
        let events = Arc::clone(&self.events);
        model.expect_locked_package_status().returning(move |_, _| {
            events.lock().unwrap().push("walk".to_string());
            Ok(status.clone())
        });
        expect_downloads(&mut model, &self.events, self.home(), self.gone);
        let home = self.home();
        model
            .expect_package_home()
            .returning(move |_| Ok(home.clone()));
        model.expect_resolve_workflow().returning(|_, _| Ok(None));
        let published = Arc::clone(&self.published);
        let events = Arc::clone(&self.events);
        let invocations = self.dir.path().join("task_invocations.json");
        model
            .expect_locked_package_publish()
            .returning(move |_, message, meta, _, _, status| {
                let UserMeta::Set(meta) = meta else {
                    panic!("a task revision sets its metadata");
                };
                // The mock is synchronous, so the file is read without the runtime.
                let own_at_push = std::fs::read(&invocations)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|file| file["invocation_ids"].as_array().cloned())
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect();
                events.lock().unwrap().push("publish".to_string());
                published.lock().unwrap().push(Revision {
                    meta,
                    message,
                    own_at_push,
                    paths: changed_paths(status.as_ref()),
                });
                Ok(quilt::PublishOutcome::CommittedAndPushed(
                    quilt::PushOutcome {
                        manifest_uri: quilt_uri::ManifestUri {
                            bucket: "quilt-dev".to_string(),
                            namespace: namespace(),
                            hash: "h1".to_string(),
                            origin: Some(fixtures::host()),
                        },
                        certified_latest,
                    },
                ))
            });
        model
    }
}

#[tokio::test]
async fn a_claim_changes_only_the_claim_and_starts_the_run() {
    let fixture = Fixture::new();

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        [
            "download TASK.md",
            "download sub/TASK.md",
            "walk",
            "publish"
        ],
        "the agent finds the whole revision on disk, and the claim publishes a tree that has it"
    );
    let published = fixture.published.lock().unwrap();
    assert_eq!(published.len(), 1, "one claim revision");
    let Revision {
        meta: claimed,
        own_at_push,
        ..
    } = &published[0];
    assert_eq!(without_claim(claimed), without_claim(&unclaimed()));
    let claim = &claimed["agent_exec"]["claim"];
    assert_eq!(claim["status"], "claimed");
    let invocation_id = claim["invocation_id"].as_str().expect("an invocation ID");
    assert!(uuid::Uuid::parse_str(invocation_id).is_ok());
    let claimed_at = claim["claimed_at"].as_str().expect("a claim time");
    assert!(chrono::DateTime::parse_from_rfc3339(claimed_at).is_ok());
    assert!(
        own_at_push.contains(invocation_id),
        "the claim was on disk as ours before it was pushed"
    );

    let runs = fixture.runs.lock().unwrap();
    assert_eq!(runs.len(), 1, "the run starts");
    assert_eq!(runs[0].document, "TASK.md");
    assert_eq!(runs[0].claim_hash, "h1");
    assert!(
        fixture.tasks.holds(&namespace()),
        "autosync is held while it runs"
    );
}

/// The download refuses to write over a file that is already there.
#[tokio::test]
async fn a_file_already_in_the_folder_is_not_downloaded_again() -> Result<(), Error> {
    let fixture = Fixture::new();
    tokio::fs::create_dir_all(fixture.home()).await?;
    tokio::fs::write(fixture.home().join("TASK.md"), "the task").await?;

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["download sub/TASK.md", "walk", "publish"]
    );
    assert_eq!(fixture.runs.lock().unwrap().len(), 1, "the run starts");
    Ok(())
}

/// Asking for one would fail the whole download: the engine refuses a path it
/// already tracks.
#[tokio::test]
async fn a_path_this_copy_tracks_is_not_downloaded_again() {
    let fixture = Fixture {
        tracked: &["TASK.md"],
        ..Fixture::new()
    };

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["download sub/TASK.md", "walk", "publish"]
    );
}

/// The download comes before the claim, outside any permission check.
#[tokio::test]
async fn a_revision_listing_a_path_outside_the_package_is_not_claimed() {
    let fixture = Fixture {
        rows: &["TASK.md", "../outside.csv"],
        ..Fixture::new()
    };

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        [] as [&str; 0],
        "nothing downloaded"
    );
    assert_eq!(fixture.metas(), [] as [Value; 0], "nothing is claimed");
}

#[tokio::test]
async fn a_task_input_whose_bytes_are_gone_stops_the_claim() {
    let fixture = Fixture {
        gone: &["sub/TASK.md"],
        ..Fixture::new()
    };

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(fixture.metas(), [] as [Value; 0], "nothing is claimed");
    assert!(fixture.runs.lock().unwrap().is_empty(), "nothing runs");
}

#[tokio::test]
async fn a_task_input_that_cannot_be_downloaded_stops_the_claim() -> Result<(), Error> {
    let fixture = Fixture::new();
    tokio::fs::create_dir_all(fixture.home().join("sub/TASK.md")).await?;

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(fixture.metas(), [] as [Value; 0], "nothing is claimed");
    assert!(fixture.runs.lock().unwrap().is_empty(), "nothing runs");
    Ok(())
}

#[tokio::test]
async fn a_folder_named_like_the_document_is_no_document() -> Result<(), Error> {
    let fixture = Fixture::new();
    tokio::fs::create_dir_all(fixture.home().join("TASK.md")).await?;

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["download TASK.md"],
        "the revision's document is asked for, and the download refuses the folder"
    );
    assert!(fixture.runs.lock().unwrap().is_empty(), "nothing runs");
    Ok(())
}

/// The walk skips symlinks, so a linked document would pass it unread.
#[cfg(unix)]
#[tokio::test]
async fn a_document_reached_through_a_symlink_is_no_document() -> Result<(), Error> {
    // The document itself a link, and the document under a linked folder.
    for (document, link) in [("TASK.md", "TASK.md"), ("sub/TASK.md", "sub")] {
        let fixture = Fixture::new();
        let elsewhere = fixture.dir.path().join("elsewhere");
        tokio::fs::create_dir_all(&elsewhere).await?;
        tokio::fs::write(elsewhere.join("TASK.md"), "other orders").await?;
        tokio::fs::create_dir_all(fixture.home()).await?;
        let target = if link == "sub" {
            elsewhere.clone()
        } else {
            elsewhere.join("TASK.md")
        };
        tokio::fs::symlink(target, fixture.home().join(link)).await?;
        let mut head = unclaimed();
        head["agent_exec"]["document"] = json!(document);

        fixture.check(head, fresh(BTreeMap::new()), true).await;

        assert_eq!(
            fixture.metas(),
            [] as [Value; 0],
            "{document}: nothing is claimed"
        );
        assert!(
            !fixture
                .events
                .lock()
                .unwrap()
                .contains(&"download sub/TASK.md".to_string()),
            "nothing is downloaded through the linked folder"
        );
    }
    Ok(())
}

/// The loser's claim changed only the metadata, so the remote wins and the
/// package is not left Diverged.
#[tokio::test]
async fn a_claim_that_did_not_become_latest_resets_to_latest_and_starts_no_run() {
    let fixture = Fixture::new();

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), false)
        .await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        [
            "download TASK.md",
            "download sub/TASK.md",
            "walk",
            "publish",
            "walk",
            "reset"
        ],
        "the claim was pushed, then the clean package went back to latest"
    );
    assert!(fixture.runs.lock().unwrap().is_empty(), "nothing runs");
    assert!(
        !fixture.tasks.holds(&namespace()),
        "and autosync is let back in"
    );
}

fn ours_claimed() -> Value {
    let mut head = unclaimed();
    head["agent_exec"]["claim"] =
        json!({ "status": "claimed", "invocation_id": "ours", "claimed_at": "t" });
    head
}

#[tokio::test]
async fn our_own_interrupted_claim_is_published_failed_with_what_the_agent_left()
-> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;

    fixture.check(ours_claimed(), fresh(written()), true).await;

    let published = fixture.published.lock().unwrap();
    assert_eq!(published.len(), 1);
    let claim = &published[0].meta["agent_exec"]["claim"];
    assert_eq!(claim["status"], "failed");
    assert_eq!(claim["reason"], "interrupted by restart");
    assert_eq!(claim["invocation_id"], "ours");
    assert_eq!(
        claim["changes"],
        json!({ "added": 1, "modified": 0, "removed": 0 })
    );
    assert_eq!(published[0].paths, BTreeSet::from(["out.md".to_string()]));
    assert!(fixture.runs.lock().unwrap().is_empty(), "no second run");
    Ok(())
}

/// The run saved its outcome, then its publish never landed, or landed outside
/// `latest` and was reset onto a `latest` still holding the claim.
#[tokio::test]
async fn the_sweep_publishes_the_outcome_the_run_saved() -> Result<(), Error> {
    let not_started = json!({ "status": "not_started", "invocation_id": "ours", "claimed_at": "t", "reason": "agent not signed in" });
    let done = json!({ "status": "done", "invocation_id": "ours", "claimed_at": "t", "session_id": "s1", "stop_reason": "end_turn" });
    let mut done_with_counts = done.clone();
    done_with_counts["changes"] = json!({ "added": 1, "modified": 0, "removed": 0 });
    for (saved, changes, published_claim, message) in [
        (
            not_started.clone(),
            BTreeMap::new(),
            not_started,
            "Task TASK.md not_started",
        ),
        (done, written(), done_with_counts, "Task TASK.md done"),
    ] {
        let fixture = Fixture::new();
        settings::remember_invocation(fixture.dir.path(), "ours").await?;
        settings::remember_outcome(fixture.dir.path(), "ours", &saved).await?;
        let paths = changes.keys().map(|p| p.display().to_string()).collect();

        fixture.check(ours_claimed(), fresh(changes), true).await;

        let published = fixture.published.lock().unwrap();
        assert_eq!(published.len(), 1, "one revision, the saved outcome's");
        assert_eq!(
            published[0].meta,
            agent_exec::with_claim(&ours_claimed(), published_claim),
            "the head's metadata, with only the claim moved on"
        );
        assert_eq!(published[0].message, message);
        assert_eq!(published[0].paths, paths);
    }
    Ok(())
}

/// The app was killed while the agent ran, and the agent kept running.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn the_sweep_stops_an_agent_left_running_before_it_publishes() -> Result<(), Error> {
    use std::os::unix::process::ExitStatusExt;

    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let mut agent = std::process::Command::new("sleep").arg("60").spawn()?;
    let spawned = agent_process::Spawned::of(agent.id()).expect("its start time");
    settings::record_agent(fixture.dir.path(), "ours", Some(spawned)).await?;

    fixture.check(ours_claimed(), fresh(written()), true).await;

    let stopped = agent.try_wait()?.and_then(|status| status.signal());
    assert_eq!(stopped, Some(libc::SIGTERM), "the agent is stopped");
    assert_eq!(
        settings::recorded_agent(fixture.dir.path(), "ours").await?,
        None,
        "and its record dropped"
    );
    assert_eq!(
        fixture.metas()[0]["agent_exec"]["claim"]["status"],
        "failed"
    );
    Ok(())
}

/// A folder in place of the file makes it unreadable.
#[tokio::test]
async fn the_sweep_publishes_failed_when_the_saved_outcomes_cannot_be_read() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    tokio::fs::create_dir(fixture.dir.path().join("task_outcomes.json")).await?;

    fixture.check(ours_claimed(), fresh(written()), true).await;

    let published = fixture.metas();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["agent_exec"]["claim"]["status"], "failed");
    Ok(())
}

/// The claim's own commit never left here, and `latest` already carries the
/// run's outcome.
#[tokio::test]
async fn a_saved_outcome_already_in_latest_is_not_published_again() -> Result<(), Error> {
    let saved = json!({ "status": "not_started", "invocation_id": "ours", "claimed_at": "t" });
    let fixture = Fixture {
        pending_commit: true,
        latest_meta: Some(agent_exec::with_claim(&ours_claimed(), saved.clone())),
        ..Fixture::new()
    };
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    settings::remember_outcome(fixture.dir.path(), "ours", &saved).await?;
    let ahead = quilt::lineage::InstalledPackageStatus {
        upstream_state: UpstreamState::Ahead,
        ..fresh(written())
    };

    fixture.check(ours_claimed(), ahead, true).await;

    assert_eq!(fixture.metas(), [] as [Value; 0]);
    Ok(())
}

/// A commit with changes carries the new metadata, so the claim fails at once.
#[tokio::test]
async fn an_interrupted_claim_with_a_pending_commit_and_changes_is_published_failed()
-> Result<(), Error> {
    let fixture = Fixture {
        pending_commit: true,
        ..Fixture::new()
    };
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let ahead = quilt::lineage::InstalledPackageStatus {
        upstream_state: UpstreamState::Ahead,
        ..fresh(written())
    };

    fixture.check(ours_claimed(), ahead, true).await;

    let published = fixture.metas();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["agent_exec"]["claim"]["status"], "failed");
    Ok(())
}

/// Publishing would push the commit and drop the new metadata.
#[tokio::test]
async fn an_interrupted_claim_on_a_pending_commit_alone_is_pushed_as_it_is() -> Result<(), Error> {
    let fixture = Fixture {
        pending_commit: true,
        ..Fixture::new()
    };
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let ahead = quilt::lineage::InstalledPackageStatus {
        upstream_state: UpstreamState::Ahead,
        ..fresh(BTreeMap::new())
    };

    fixture.check(ours_claimed(), ahead.clone(), true).await;
    assert_eq!(*fixture.events.lock().unwrap(), ["walk", "push"]);

    let losing = Fixture {
        pending_commit: true,
        ..Fixture::new()
    };
    settings::remember_invocation(losing.dir.path(), "ours").await?;
    losing.check(ours_claimed(), ahead, false).await;
    assert_eq!(
        *losing.events.lock().unwrap(),
        ["walk", "push"],
        "what the commit holds is unknown, so a push latest didn't take is not reset"
    );
    Ok(())
}

/// The claim's own commit never left here, and `latest` doesn't hold it.
async fn sweep_lost_claim(
    stacked_commits: bool,
    changes: BTreeMap<PathBuf, quilt::lineage::Change>,
) -> Vec<String> {
    let fixture = Fixture {
        pending_commit: true,
        stacked_commits,
        latest_meta: Some(unclaimed()),
        ..Fixture::new()
    };
    settings::remember_invocation(fixture.dir.path(), "ours")
        .await
        .expect("remembered");
    let ahead = quilt::lineage::InstalledPackageStatus {
        upstream_state: UpstreamState::Ahead,
        ..fresh(changes)
    };
    fixture.check(ours_claimed(), ahead, true).await;
    assert_eq!(fixture.metas(), [] as [Value; 0], "no failed revision");
    fixture.events.lock().unwrap().clone()
}

#[tokio::test]
async fn a_lost_claim_resets_when_its_own_commit_is_all_there_is() {
    assert_eq!(
        sweep_lost_claim(false, BTreeMap::new()).await,
        ["walk", "reset"]
    );
}

#[tokio::test]
async fn a_lost_claim_stays_diverged_over_a_later_commit_or_changes() {
    assert_eq!(sweep_lost_claim(true, BTreeMap::new()).await, ["walk"]);
    assert_eq!(sweep_lost_claim(false, written()).await, ["walk"]);
}

#[tokio::test]
async fn an_interrupted_claim_waits_while_latest_cannot_be_read() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let stale = quilt::lineage::InstalledPackageStatus {
        latest_refreshed: false,
        ..fresh(written())
    };

    fixture.check(ours_claimed(), stale, true).await;

    assert_eq!(fixture.metas(), [] as [Value; 0]);
    Ok(())
}

#[tokio::test]
async fn autosync_leaves_our_own_interrupted_claim_to_the_sweep() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let pkg = quilt::LocalDomain::new(PathBuf::new()).create_installed_package(namespace());
    let awaits =
        async |model: &MockQuiltModel| fixture.tasks.awaits_sweep(model, &settings(), &pkg).await;

    let ours = fixture.model(ours_claimed(), fresh(written()), true);
    assert!(awaits(&ours).await);
    let theirs = fixture.model(unclaimed(), fresh(written()), true);
    assert!(!awaits(&theirs).await);
    // The stored lineage still says up to date; only a fresh read finds it behind.
    let ours_behind = fixture.model(
        ours_claimed(),
        quilt::lineage::InstalledPackageStatus {
            upstream_state: UpstreamState::Behind,
            ..fresh(written())
        },
        true,
    );
    assert!(
        !awaits(&ours_behind).await,
        "behind latest, the sweep can't publish it, so autosync pulls as usual"
    );
    let unread = fixture.model(
        ours_claimed(),
        quilt::lineage::InstalledPackageStatus {
            latest_refreshed: false,
            ..fresh(written())
        },
        true,
    );
    assert!(
        awaits(&unread).await,
        "while latest can't be read, the sweep waits and autosync with it"
    );
    let _running = fixture.tasks.hold(&namespace());
    assert!(
        !awaits(&ours).await,
        "a live run is the hold's, not the sweep's"
    );
    Ok(())
}

#[tokio::test]
async fn our_own_claim_with_its_run_alive_is_left_to_the_run() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let mut head = unclaimed();
    head["agent_exec"]["claim"] = json!({ "status": "claimed", "invocation_id": "ours" });
    let _running = fixture.tasks.hold(&namespace());

    fixture.check(head, fresh(BTreeMap::new()), true).await;

    assert!(fixture.metas().is_empty(), "the run publishes its own end");
    Ok(())
}

#[tokio::test]
async fn a_task_whose_document_is_missing_is_published_unrunnable() {
    let fixture = Fixture::new();
    let mut head = unclaimed();
    head["agent_exec"]["document"] = json!("MISSING.md");

    fixture.check(head, fresh(BTreeMap::new()), true).await;

    let published = fixture.metas();
    assert_eq!(published.len(), 1);
    let claim = &published[0]["agent_exec"]["claim"];
    assert_eq!(claim["status"], "unrunnable");
    assert!(
        claim["reason"]
            .as_str()
            .is_some_and(|r| r.contains("MISSING.md"))
    );
    assert!(fixture.runs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unrunnable_that_is_not_latest_resets_to_latest() {
    let fixture = Fixture::new();
    let mut head = unclaimed();
    head["agent_exec"]["document"] = json!("MISSING.md");

    fixture.check(head, fresh(BTreeMap::new()), false).await;

    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["walk", "publish", "walk", "reset"]
    );
}

/// A claim publishes only the metadata, which a tree with local changes
/// would sweep in.
#[tokio::test]
async fn a_task_on_a_package_with_local_changes_waits() {
    let fixture = Fixture::new();
    let changes = BTreeMap::from([(
        PathBuf::from("notes.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    )]);

    fixture.check(unclaimed(), fresh(changes), true).await;

    assert!(fixture.metas().is_empty(), "nothing is claimed");
    assert!(fixture.runs.lock().unwrap().is_empty());
}

/// A tree that only looks up to date because the remote could not be reached.
#[tokio::test]
async fn a_task_waits_while_latest_cannot_be_read() {
    let fixture = Fixture::new();
    let stale = quilt::lineage::InstalledPackageStatus {
        latest_refreshed: false,
        ..fresh(BTreeMap::new())
    };

    fixture.check(unclaimed(), stale, true).await;

    assert!(fixture.metas().is_empty(), "nothing is claimed");
}

/// A package whose remote `latest` reads as `latest_hash` (fresh unless
/// `refreshed` is false), carries `latest_meta` and lists `TASK.md` and
/// `NEXT.md`. Pulls and downloads go to the fixture's events.
fn rereading_model(
    fixture: &Fixture,
    latest_hash: &str,
    refreshed: bool,
    latest_meta: Value,
) -> MockQuiltModel {
    let events = &fixture.events;
    let mut model = MockQuiltModel::new();
    let home = fixture.home();
    model
        .expect_package_home()
        .returning(move |_| Ok(home.clone()));
    expect_downloads(&mut model, events, fixture.home(), fixture.gone);
    model.expect_get_installed_package().returning(|ns| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new()).create_installed_package(ns.clone()),
        ))
    });
    let status = quilt::lineage::InstalledPackageStatus {
        latest_refreshed: refreshed,
        latest_hash: latest_hash.to_string(),
        ..fresh(BTreeMap::new())
    };
    model
        .expect_get_installed_package_status()
        .returning(move |_, _| Ok(status.clone()));
    let remote = quilt_uri::ManifestUri {
        bucket: "quilt-dev".to_string(),
        namespace: namespace(),
        hash: "h1".to_string(),
        origin: Some(fixtures::host()),
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h1".to_string());
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage.clone()));
    let pulled_meta = fixture.pulled_meta.clone();
    let landed = if pulled_meta.is_some() { "h3" } else { "h2" };
    model.expect_browse_remote_manifest().returning(move |uri| {
        let meta = match &pulled_meta {
            Some(pulled) if uri.hash == "h3" => pulled,
            _ => &latest_meta,
        };
        Ok(manifest(meta, &["TASK.md", "NEXT.md"]))
    });
    model
        .expect_lock_package()
        .returning(|p| Ok(p.namespace.clone()));
    let pulled = Arc::clone(events);
    model
        .expect_locked_package_pull()
        .returning(move |_, _, _| {
            pulled.lock().unwrap().push("pull".to_string());
            Ok(quilt::flow::PullReport {
                manifest_uri: quilt_uri::ManifestUri {
                    bucket: "quilt-dev".to_string(),
                    namespace: namespace(),
                    hash: landed.to_string(),
                    origin: None,
                },
                added: Vec::new(),
                added_not_fetched: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                message: None,
            })
        });
    model
}

/// The run's claim, `inv`, as `claimed_run` writes it.
fn claimed_by_this_run() -> Value {
    agent_exec::with_claim(
        &unclaimed(),
        json!({ "status": "claimed", "invocation_id": "inv", "claimed_at": "t" }),
    )
}

async fn recheck_with(
    fixture: &Fixture,
    model: &MockQuiltModel,
) -> Result<String, acp::RunOutcome> {
    recheck_claim(model, &claimed_run(&fixture.tasks))
        .await
        .expect("latest was read")
}

#[tokio::test]
async fn the_claim_still_at_latest_runs_as_it_is() {
    let fixture = Fixture::new();
    let model = rereading_model(&fixture, "h1", true, claimed_by_this_run());

    assert_eq!(
        recheck_with(&fixture, &model).await,
        Ok("TASK.md".to_string())
    );
    assert!(fixture.events.lock().unwrap().is_empty(), "nothing to pull");
}

/// The later revision moved the task on to another document.
#[tokio::test]
async fn a_later_revision_that_still_holds_the_claim_is_pulled_and_its_task_run() {
    let fixture = Fixture::new();
    let mut later = claimed_by_this_run();
    later["agent_exec"]["document"] = json!("NEXT.md");
    let model = rereading_model(&fixture, "h2", true, later);

    assert_eq!(
        recheck_with(&fixture, &model).await,
        Ok("NEXT.md".to_string())
    );
    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["pull", "download TASK.md", "download NEXT.md"]
    );
}

#[tokio::test]
async fn a_later_revision_whose_inputs_cannot_be_downloaded_fails_the_reread() -> Result<(), Error>
{
    let fixture = Fixture::new();
    tokio::fs::create_dir_all(fixture.home().join("TASK.md")).await?;
    let mut later = claimed_by_this_run();
    later["agent_exec"]["document"] = json!("NEXT.md");
    let model = rereading_model(&fixture, "h2", true, later);

    let reread = recheck_claim(&model, &claimed_run(&fixture.tasks)).await;

    assert!(
        reread
            .as_ref()
            .is_err_and(|err| err.to_string().contains("TASK.md")),
        "{reread:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_task_changed_to_another_agent_is_not_started_here() {
    let fixture = Fixture::new();
    let mut later = claimed_by_this_run();
    later["agent_exec"]["agent"]["id"] = json!("claude");
    // The agent is checked before the document, as Detect checks them.
    later["agent_exec"]["document"] = json!("MISSING.md");
    let model = rereading_model(&fixture, "h2", true, later);

    assert_eq!(
        recheck_with(&fixture, &model).await,
        Err(acp::RunOutcome::NotStarted {
            reason: "task changed to an agent not running here".to_string()
        })
    );
    assert_eq!(*fixture.events.lock().unwrap(), ["pull"]);
}

#[tokio::test]
async fn a_pull_that_lands_a_revision_without_the_claim_stops_the_run() {
    let mut theirs = claimed_by_this_run();
    theirs["agent_exec"]["claim"]["invocation_id"] = json!("theirs");
    let fixture = Fixture {
        pulled_meta: Some(theirs),
        ..Fixture::new()
    };
    let model = rereading_model(&fixture, "h2", true, claimed_by_this_run());

    assert_eq!(
        recheck_with(&fixture, &model).await,
        Err(acp::RunOutcome::Superseded)
    );
    assert_eq!(*fixture.events.lock().unwrap(), ["pull"]);
}

#[tokio::test]
async fn a_task_changed_to_a_document_not_in_the_package_is_unrunnable() {
    let fixture = Fixture::new();
    let mut later = claimed_by_this_run();
    later["agent_exec"]["document"] = json!("MISSING.md");
    let model = rereading_model(&fixture, "h2", true, later);

    assert!(matches!(
        recheck_with(&fixture, &model).await,
        Err(acp::RunOutcome::Unrunnable { .. })
    ));
}

#[tokio::test]
async fn a_latest_that_no_longer_holds_the_claim_stops_the_run() {
    let fixture = Fixture::new();
    let mut theirs = claimed_by_this_run();
    theirs["agent_exec"]["claim"]["invocation_id"] = json!("theirs");
    let model = rereading_model(&fixture, "h2", true, theirs);

    assert_eq!(
        recheck_with(&fixture, &model).await,
        Err(acp::RunOutcome::Superseded)
    );
    assert!(
        fixture.events.lock().unwrap().is_empty(),
        "nothing is pulled"
    );
}

#[tokio::test]
async fn an_unread_latest_is_no_answer() {
    let fixture = Fixture::new();
    let model = rereading_model(&fixture, "h1", false, claimed_by_this_run());

    assert!(
        recheck_claim(&model, &claimed_run(&fixture.tasks))
            .await
            .is_err()
    );
}

fn claimed_run(tasks: &Tasks) -> Run {
    let claim = Claim {
        invocation_id: "inv".to_string(),
        claimed_at: "t".to_string(),
    };
    Run {
        namespace: namespace(),
        home: PathBuf::new(),
        document: "TASK.md".to_string(),
        agent_id: "kiro".to_string(),
        agent: settings().agents["kiro"].clone(),
        claim,
        claim_hash: "h1".to_string(),
        data_dir: tasks.data_dir.clone().expect("tasks on"),
        _hold: tasks.hold(&namespace()),
    }
}

/// A package the run ended on: its head carries the fixture's `finished_meta`
/// under the workflow `edited`, its tree walks with `changes`, and its publishes are taken as
/// `latest` or not by `certified_latest`. Publishes go to the fixture's
/// `published`; publishes and resets to its `events`.
fn finishing_model(
    fixture: &Fixture,
    changes: BTreeMap<PathBuf, quilt::lineage::Change>,
    certified_latest: bool,
) -> MockQuiltModel {
    let mut model = MockQuiltModel::new();
    model.expect_get_installed_package().returning(|ns| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new()).create_installed_package(ns.clone()),
        ))
    });
    model
        .expect_lock_package()
        .returning(|p| Ok(p.namespace.clone()));
    // The publish commits the tree's changes, so later walks find it clean.
    let committed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let walked = Arc::clone(&committed);
    let walk_fails = fixture.walk_fails;
    model.expect_locked_package_status().returning(move |_, _| {
        if walk_fails {
            Err(Error::General("signed out".to_string()))
        } else if walked.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(fresh(BTreeMap::new()))
        } else {
            Ok(fresh(changes.clone()))
        }
    });
    // A pending commit, as `Fixture::lineage` writes it, until the publish pushes it.
    let pending = fixture
        .pending_commit
        .then(|| fixture.lineage().commit)
        .flatten();
    let walked = Arc::clone(&committed);
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| {
            let mut lineage = quilt::lineage::PackageLineage::default();
            if !walked.load(std::sync::atomic::Ordering::SeqCst) {
                lineage.commit.clone_from(&pending);
            }
            Ok(lineage)
        });
    let head_meta = fixture.finished_meta.clone().unwrap_or_else(edited_head);
    model
        .expect_get_installed_package_manifest()
        .returning(move |_, _| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    user_meta: Some(head_meta.clone()),
                    workflow: Some(quilt::manifest::Workflow {
                        config: quilt_uri::S3Uri {
                            bucket: "quilt-dev".to_string(),
                            key: ".quilt/workflows/config.yaml".to_string(),
                            version: None,
                        },
                        id: Some(quilt::manifest::WorkflowId {
                            id: "edited".to_string(),
                            schemas: BTreeMap::new(),
                        }),
                    }),
                    ..quilt::manifest::ManifestHeader::default()
                },
                rows: Vec::new(),
            })
        });
    model
        .expect_resolve_workflow()
        .withf(|_, intent| *intent == WorkflowIntent::Named("edited".to_string()))
        .returning(|_, _| Ok(None));
    expect_push_and_reset(&mut model, &fixture.events, certified_latest);
    let published = Arc::clone(&fixture.published);
    let events = Arc::clone(&fixture.events);
    model
        .expect_locked_package_publish()
        .returning(move |_, message, meta, _, _, status| {
            let UserMeta::Set(meta) = meta else {
                panic!("a task revision sets its metadata");
            };
            events.lock().unwrap().push("publish".to_string());
            committed.store(true, std::sync::atomic::Ordering::SeqCst);
            published.lock().unwrap().push(Revision {
                meta,
                message,
                own_at_push: BTreeSet::new(),
                paths: changed_paths(status.as_ref()),
            });
            Ok(quilt::PublishOutcome::CommittedAndPushed(
                quilt::PushOutcome {
                    manifest_uri: quilt_uri::ManifestUri {
                        bucket: "quilt-dev".to_string(),
                        namespace: namespace(),
                        hash: "h3".to_string(),
                        origin: None,
                    },
                    certified_latest,
                },
            ))
        });
    model
}

/// The head a run ends on, after the re-read pulled a later revision that
/// still holds its claim: someone edited the title there.
fn edited_head() -> Value {
    let mut head = claimed_by_this_run();
    head["title"] = json!("edited");
    head
}

fn not_started() -> acp::RunOutcome {
    acp::RunOutcome::NotStarted {
        reason: "agent not signed in".to_string(),
    }
}

#[tokio::test]
async fn the_last_revision_publishes_the_agents_work_under_the_same_claim() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, written(), true);
    let done = acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    };

    finish(&model, &claimed_run(&fixture.tasks), &done).await?;

    let published = fixture.published.lock().unwrap();
    assert_eq!(published.len(), 1);
    let Revision { meta, paths, .. } = &published[0];
    assert_eq!(
        *meta,
        agent_exec::with_claim(
            &edited_head(),
            json!({ "status": "done", "invocation_id": "inv", "claimed_at": "t",
                    "session_id": "s1", "stop_reason": "end_turn",
                    "changes": { "added": 1, "modified": 0, "removed": 0 } })
        ),
        "the head's own metadata, with only the claim moved on"
    );
    assert_eq!(published[0].message, "Task TASK.md done");
    assert_eq!(*paths, BTreeSet::from(["out.md".to_string()]));
    Ok(())
}

fn done() -> acp::RunOutcome {
    acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    }
}

/// One file of each kind of change.
fn each_change() -> BTreeMap<PathBuf, quilt::lineage::Change> {
    let row = quilt::manifest::ManifestRow::default;
    BTreeMap::from([
        (PathBuf::from("a.md"), quilt::lineage::Change::Added(row())),
        (PathBuf::from("b.md"), quilt::lineage::Change::Added(row())),
        (
            PathBuf::from("c.md"),
            quilt::lineage::Change::Modified(row()),
        ),
        (
            PathBuf::from("d.md"),
            quilt::lineage::Change::Removed(row()),
        ),
    ])
}

#[tokio::test]
async fn the_last_revision_counts_what_the_run_changed() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, each_change(), true);

    finish(&model, &claimed_run(&fixture.tasks), &done()).await?;

    let published = fixture.published.lock().unwrap();
    assert_eq!(
        published[0].meta["agent_exec"]["claim"]["changes"],
        json!({ "added": 2, "modified": 1, "removed": 1 })
    );
    assert_eq!(published[0].message, "Task TASK.md done");
    Ok(())
}

#[tokio::test]
async fn a_run_that_changed_nothing_says_so() -> Result<(), Error> {
    let failed = acp::RunOutcome::Failed {
        session_id: None,
        stop_reason: Some("max_tokens".to_string()),
        reason: None,
    };
    for (outcome, message) in [
        (done(), "Task TASK.md done, no changes"),
        (failed, "Task TASK.md failed, no changes"),
    ] {
        let fixture = Fixture::new();
        let model = finishing_model(&fixture, BTreeMap::new(), true);

        finish(&model, &claimed_run(&fixture.tasks), &outcome).await?;

        let published = fixture.published.lock().unwrap();
        let claim = &published[0].meta["agent_exec"]["claim"];
        assert_eq!(
            claim["changes"],
            json!({ "added": 0, "modified": 0, "removed": 0 })
        );
        assert_eq!(published[0].message, message);
    }
    Ok(())
}

/// The walk compares against the commit, not the claimed revision.
#[tokio::test]
async fn a_run_over_a_commit_made_during_it_counts_no_changes() -> Result<(), Error> {
    let fixture = Fixture {
        pending_commit: true,
        ..Fixture::new()
    };
    let model = finishing_model(&fixture, written(), true);

    finish(&model, &claimed_run(&fixture.tasks), &done()).await?;

    let published = fixture.published.lock().unwrap();
    assert_eq!(published[0].meta["agent_exec"]["claim"]["status"], "done");
    assert!(
        published[0].meta["agent_exec"]["claim"]
            .get("changes")
            .is_none()
    );
    assert_eq!(published[0].message, "Task TASK.md done");
    Ok(())
}

#[tokio::test]
async fn a_not_started_counts_no_changes() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, BTreeMap::new(), true);

    finish(&model, &claimed_run(&fixture.tasks), &not_started()).await?;

    let published = fixture.published.lock().unwrap();
    assert!(
        published[0].meta["agent_exec"]["claim"]
            .get("changes")
            .is_none()
    );
    assert_eq!(published[0].message, "Task TASK.md not_started");
    Ok(())
}

/// The walk fails, so the outcome is never published.
#[tokio::test]
async fn a_run_that_cannot_publish_leaves_its_outcome_saved_for_the_sweep() -> Result<(), Error> {
    let fixture = Fixture {
        walk_fails: true,
        ..Fixture::new()
    };
    let model = finishing_model(&fixture, written(), true);
    let done = acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    };
    let run = claimed_run(&fixture.tasks);

    assert!(finish(&model, &run, &done).await.is_err());

    assert_eq!(
        settings::remembered_outcome(fixture.dir.path(), "inv").await?,
        run.claim.outcome(&done)
    );
    Ok(())
}

/// A folder in place of the file makes it unwritable.
#[tokio::test]
async fn an_outcome_that_cannot_be_saved_is_still_published() -> Result<(), Error> {
    let fixture = Fixture::new();
    tokio::fs::create_dir(fixture.dir.path().join("task_outcomes.json")).await?;
    let model = finishing_model(&fixture, written(), true);
    let done = acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    };

    let finished = finish(&model, &claimed_run(&fixture.tasks), &done).await;

    assert!(finished.is_ok(), "{finished:?}");
    assert_eq!(fixture.metas()[0]["agent_exec"]["claim"]["status"], "done");
    Ok(())
}

/// The user pulled a revision where the author cancelled the task.
#[tokio::test]
async fn a_head_that_no_longer_holds_the_claim_gets_no_outcome() -> Result<(), Error> {
    let mut cancelled = edited_head();
    cancelled["agent_exec"]["claim"] = json!({ "status": "cancelled" });
    let done = acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    };
    // A clean `not_started` would otherwise publish and reset.
    for (outcome, changes) in [(done, written()), (not_started(), BTreeMap::new())] {
        let fixture = Fixture {
            finished_meta: Some(cancelled.clone()),
            ..Fixture::new()
        };
        let model = finishing_model(&fixture, changes, false);

        finish(&model, &claimed_run(&fixture.tasks), &outcome).await?;

        assert!(
            fixture.events.lock().unwrap().is_empty(),
            "{outcome:?}: nothing published or reset, the files stay for autosync"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_terminal_revision_that_is_not_latest_is_left_diverged() -> Result<(), Error> {
    let fixture = Fixture::new();
    // A clean tree, which alone would not stop a reset.
    let model = finishing_model(&fixture, BTreeMap::new(), false);
    let done = acp::RunOutcome::Done {
        session_id: "s1".to_string(),
        stop_reason: "end_turn".to_string(),
    };

    finish(&model, &claimed_run(&fixture.tasks), &done).await?;

    assert_eq!(*fixture.events.lock().unwrap(), ["publish"], "no reset");
    Ok(())
}

#[tokio::test]
async fn a_not_started_that_is_not_latest_resets_while_the_tree_is_clean() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, BTreeMap::new(), false);

    finish(&model, &claimed_run(&fixture.tasks), &not_started()).await?;

    assert_eq!(*fixture.events.lock().unwrap(), ["publish", "reset"]);
    Ok(())
}

/// The user committed by hand during the run; the publish pushes that commit.
#[tokio::test]
async fn a_not_started_over_a_commit_made_during_the_run_is_never_reset() -> Result<(), Error> {
    let fixture = Fixture {
        pending_commit: true,
        ..Fixture::new()
    };
    let model = finishing_model(&fixture, BTreeMap::new(), false);

    finish(&model, &claimed_run(&fixture.tasks), &not_started()).await?;

    assert_eq!(*fixture.events.lock().unwrap(), ["publish"], "no reset");
    Ok(())
}

/// The agent wrote files before its sign-in failed.
#[tokio::test]
async fn a_not_started_that_is_not_latest_stays_diverged_over_a_changed_tree() -> Result<(), Error>
{
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, written(), false);

    finish(&model, &claimed_run(&fixture.tasks), &not_started()).await?;

    assert_eq!(*fixture.events.lock().unwrap(), ["publish"], "no reset");
    Ok(())
}

/// The re-read found the task changed into one that can't run.
#[tokio::test]
async fn an_unrunnable_found_at_the_reread_is_published_and_resets_if_not_latest()
-> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, BTreeMap::new(), false);
    let unrunnable = acp::RunOutcome::Unrunnable {
        reason: "MISSING.md is not a file in the package".to_string(),
    };

    finish(&model, &claimed_run(&fixture.tasks), &unrunnable).await?;

    assert_eq!(
        fixture.metas()[0]["agent_exec"]["claim"]["status"],
        "unrunnable"
    );
    assert_eq!(*fixture.events.lock().unwrap(), ["publish", "reset"]);
    Ok(())
}

#[tokio::test]
async fn a_run_stopped_before_the_prompt_publishes_nothing_and_resets() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = finishing_model(&fixture, BTreeMap::new(), true);

    finish(
        &model,
        &claimed_run(&fixture.tasks),
        &acp::RunOutcome::Superseded,
    )
    .await?;

    assert_eq!(*fixture.events.lock().unwrap(), ["reset"]);
    Ok(())
}

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
    /// The package's last-known `latest` is past its base.
    behind: bool,
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
            behind: false,
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
        let latest = if self.behind { "h9" } else { "h0" };
        let mut lineage = quilt::lineage::PackageLineage::from_remote(remote, latest.to_string());
        if self.pending_commit {
            lineage.commit = Some(quilt::lineage::CommitState {
                timestamp: chrono::Utc::now(),
                hash: "c1".to_string(),
                prev_hashes: Vec::new(),
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

    /// Check a package in a blessed bucket whose head carries `meta` and the
    /// file `TASK.md`, not yet downloaded, whose tree walks as `status`,
    /// publishing with `certified_latest`.
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
        model
            .expect_get_installed_package_manifest()
            .returning(move |_, _| {
                Ok(quilt::manifest::Manifest {
                    header: quilt::manifest::ManifestHeader {
                        user_meta: Some(meta.clone()),
                        ..quilt::manifest::ManifestHeader::default()
                    },
                    rows: ["TASK.md", "sub/TASK.md"]
                        .into_iter()
                        .map(|key| quilt::manifest::ManifestRow {
                            logical_key: PathBuf::from(key),
                            ..quilt::manifest::ManifestRow::default()
                        })
                        .collect(),
                })
            });
        model
            .expect_try_lock_package()
            .returning(|p| Ok(Some(p.namespace.clone())));
        let events = Arc::clone(&self.events);
        model.expect_locked_package_status().returning(move |_, _| {
            events.lock().unwrap().push("walk".to_string());
            Ok(status.clone())
        });
        let events = Arc::clone(&self.events);
        let home = self.home();
        model
            .expect_locked_package_install_paths()
            .returning(move |_, paths| {
                for path in paths {
                    events
                        .lock()
                        .unwrap()
                        .push(format!("download {}", path.display()));
                    // As the engine does, refuse to write over anything there.
                    if std::fs::symlink_metadata(home.join(path)).is_ok() {
                        return Err(Error::General(format!("{} exists", path.display())));
                    }
                }
                Ok(quilt::flow::InstallPathsReport::default())
            });
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
            .returning(move |_, _, meta, _, _, status| {
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
        ["download TASK.md", "walk", "publish"],
        "the agent finds its document on disk, and the claim publishes a tree that has it"
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
async fn a_document_already_in_the_folder_is_not_downloaded_again() -> Result<(), Error> {
    let fixture = Fixture::new();
    tokio::fs::create_dir_all(fixture.home()).await?;
    tokio::fs::write(fixture.home().join("TASK.md"), "the task").await?;

    fixture
        .check(unclaimed(), fresh(BTreeMap::new()), true)
        .await;

    assert_eq!(*fixture.events.lock().unwrap(), ["walk", "publish"]);
    assert_eq!(fixture.runs.lock().unwrap().len(), 1, "the run starts");
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
        ["download TASK.md", "walk", "publish", "walk", "reset"],
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
    assert_eq!(published[0].paths, BTreeSet::from(["out.md".to_string()]));
    assert!(fixture.runs.lock().unwrap().is_empty(), "no second run");
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
    let behind = Fixture {
        behind: true,
        ..Fixture::new()
    };
    let ours_behind = behind.model(ours_claimed(), fresh(written()), true);
    assert!(
        !awaits(&ours_behind).await,
        "behind latest, the sweep can't publish it, so autosync pulls as usual"
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
/// `refreshed` is false) and carries `latest_meta`, recording pulls in `events`.
fn rereading_model(
    latest_hash: &str,
    refreshed: bool,
    latest_meta: Value,
    events: &Arc<Mutex<Vec<String>>>,
) -> MockQuiltModel {
    let mut model = MockQuiltModel::new();
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
    model.expect_browse_remote_manifest().returning(move |_| {
        Ok(quilt::manifest::Manifest {
            header: quilt::manifest::ManifestHeader {
                user_meta: Some(latest_meta.clone()),
                ..quilt::manifest::ManifestHeader::default()
            },
            rows: Vec::new(),
        })
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
                    hash: "h2".to_string(),
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

#[tokio::test]
async fn the_claim_still_at_latest_runs_as_it_is() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = rereading_model("h1", true, claimed_by_this_run(), &fixture.events);

    assert!(claim_holds(&model, &claimed_run(&fixture.tasks)).await?);
    assert!(fixture.events.lock().unwrap().is_empty(), "nothing to pull");
    Ok(())
}

#[tokio::test]
async fn a_later_revision_that_still_holds_the_claim_is_pulled_and_run() -> Result<(), Error> {
    let fixture = Fixture::new();
    let model = rereading_model("h2", true, claimed_by_this_run(), &fixture.events);

    assert!(claim_holds(&model, &claimed_run(&fixture.tasks)).await?);
    assert_eq!(*fixture.events.lock().unwrap(), ["pull"]);
    Ok(())
}

#[tokio::test]
async fn a_latest_that_no_longer_holds_the_claim_stops_the_run() -> Result<(), Error> {
    let fixture = Fixture::new();
    let mut theirs = claimed_by_this_run();
    theirs["agent_exec"]["claim"]["invocation_id"] = json!("theirs");
    let model = rereading_model("h2", true, theirs, &fixture.events);

    assert!(!claim_holds(&model, &claimed_run(&fixture.tasks)).await?);
    assert!(
        fixture.events.lock().unwrap().is_empty(),
        "nothing is pulled"
    );
    Ok(())
}

#[tokio::test]
async fn an_unread_latest_is_no_answer() {
    let fixture = Fixture::new();
    let model = rereading_model("h1", false, claimed_by_this_run(), &fixture.events);

    assert!(
        claim_holds(&model, &claimed_run(&fixture.tasks))
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
        agent: settings().agents["kiro"].clone(),
        claimed_meta: agent_exec::with_claim(&unclaimed(), claim.claimed()),
        claim,
        claim_hash: "h1".to_string(),
        workflow: WorkflowIntent::Named("gate".to_string()),
        _hold: tasks.hold(&namespace()),
    }
}

/// A package the run ended on: its head carries the run's claim with an
/// edited title, its tree walks with `changes`, and its publishes are taken as
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
    model
        .expect_locked_package_status()
        .returning(move |_, _| Ok(fresh(changes.clone())));
    let lineage = quilt::lineage::PackageLineage::default();
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage.clone()));
    model
        .expect_get_installed_package_manifest()
        .returning(|_, _| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    user_meta: Some(edited_head()),
                    ..quilt::manifest::ManifestHeader::default()
                },
                rows: Vec::new(),
            })
        });
    model
        .expect_resolve_workflow()
        .withf(|_, intent| *intent == WorkflowIntent::Named("gate".to_string()))
        .returning(|_, _| Ok(None));
    expect_push_and_reset(&mut model, &fixture.events, certified_latest);
    let published = Arc::clone(&fixture.published);
    let events = Arc::clone(&fixture.events);
    model
        .expect_locked_package_publish()
        .returning(move |_, _, meta, _, _, status| {
            let UserMeta::Set(meta) = meta else {
                panic!("a task revision sets its metadata");
            };
            events.lock().unwrap().push("publish".to_string());
            published.lock().unwrap().push(Revision {
                meta,
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
                    "session_id": "s1", "stop_reason": "end_turn" })
        ),
        "the head's own metadata, with only the claim moved on"
    );
    assert_eq!(*paths, BTreeSet::from(["out.md".to_string()]));
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

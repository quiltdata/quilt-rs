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

/// Each revision a check published: its metadata, and the invocation IDs this
/// desktop had on disk when it was pushed.
type Published = Arc<Mutex<Vec<(Value, BTreeSet<String>)>>>;

/// Tasks in a temporary data folder that keep every run they are asked to
/// start, and every revision they publish.
struct Fixture {
    dir: TempDir,
    published: Published,
    runs: Arc<Mutex<Vec<Run>>>,
    tasks: Tasks,
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
            runs,
            tasks,
        }
    }

    fn metas(&self) -> Vec<Value> {
        let published = self.published.lock().unwrap();
        published.iter().map(|(meta, _)| meta.clone()).collect()
    }

    /// Check an up-to-date package in a blessed bucket whose head carries
    /// `meta` and the file `TASK.md`, with `changes` in its tree, publishing
    /// with `certified_latest`.
    async fn check(
        &self,
        meta: Value,
        changes: BTreeMap<PathBuf, quilt::lineage::Change>,
        certified_latest: bool,
    ) {
        let model = self.model(meta, changes, certified_latest);
        let pkg = quilt::LocalDomain::new(PathBuf::new()).create_installed_package(namespace());
        check(&model, &self.tasks, &settings(), &pkg).await;
    }

    fn model(
        &self,
        meta: Value,
        changes: BTreeMap<PathBuf, quilt::lineage::Change>,
        certified_latest: bool,
    ) -> MockQuiltModel {
        let remote = quilt_uri::ManifestUri {
            bucket: "quilt-dev".to_string(),
            namespace: namespace(),
            hash: "h0".to_string(),
            origin: Some(fixtures::host()),
        };
        let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string());
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| Ok(lineage.clone()));
        model
            .expect_get_installed_package_manifest()
            .returning(move |_, _| {
                Ok(quilt::manifest::Manifest {
                    header: quilt::manifest::ManifestHeader {
                        user_meta: Some(meta.clone()),
                        ..quilt::manifest::ManifestHeader::default()
                    },
                    rows: vec![quilt::manifest::ManifestRow {
                        logical_key: PathBuf::from("TASK.md"),
                        ..quilt::manifest::ManifestRow::default()
                    }],
                })
            });
        model
            .expect_try_lock_package()
            .returning(|p| Ok(Some(p.namespace.clone())));
        model.expect_locked_package_status().returning(move |_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::UpToDate,
                changes.clone(),
            ))
        });
        model
            .expect_package_home()
            .returning(|_| Ok(PathBuf::from("/home/u/QuiltSync/acme/demo")));
        model.expect_resolve_workflow().returning(|_, _| Ok(None));
        let published = Arc::clone(&self.published);
        let invocations = self.dir.path().join("task_invocations.json");
        model
            .expect_locked_package_publish()
            .returning(move |_, _, meta, _, _, _| {
                let UserMeta::Set(meta) = meta else {
                    panic!("a task revision sets its metadata");
                };
                // The mock is synchronous, so the file is read without the runtime.
                let own = std::fs::read(&invocations)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|file| file["invocation_ids"].as_array().cloned())
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect();
                published.lock().unwrap().push((meta, own));
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

    fixture.check(unclaimed(), BTreeMap::new(), true).await;

    let published = fixture.published.lock().unwrap();
    assert_eq!(published.len(), 1, "one claim revision");
    let (claimed, own_at_push) = &published[0];
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

#[tokio::test]
async fn a_claim_that_did_not_become_latest_starts_no_run() {
    let fixture = Fixture::new();

    fixture.check(unclaimed(), BTreeMap::new(), false).await;

    assert_eq!(fixture.metas().len(), 1, "the claim was pushed");
    assert!(fixture.runs.lock().unwrap().is_empty(), "but nothing runs");
    assert!(
        !fixture.tasks.holds(&namespace()),
        "and autosync is let back in"
    );
}

#[tokio::test]
async fn our_own_interrupted_claim_is_published_failed() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let mut head = unclaimed();
    head["agent_exec"]["claim"] =
        json!({ "status": "claimed", "invocation_id": "ours", "claimed_at": "t" });

    fixture.check(head, BTreeMap::new(), true).await;

    let published = fixture.metas();
    assert_eq!(published.len(), 1);
    let claim = &published[0]["agent_exec"]["claim"];
    assert_eq!(claim["status"], "failed");
    assert_eq!(claim["reason"], "interrupted by restart");
    assert_eq!(claim["invocation_id"], "ours");
    assert!(fixture.runs.lock().unwrap().is_empty(), "no second run");
    Ok(())
}

#[tokio::test]
async fn our_own_claim_with_its_run_alive_is_left_to_the_run() -> Result<(), Error> {
    let fixture = Fixture::new();
    settings::remember_invocation(fixture.dir.path(), "ours").await?;
    let mut head = unclaimed();
    head["agent_exec"]["claim"] = json!({ "status": "claimed", "invocation_id": "ours" });
    let _running = fixture.tasks.hold(&namespace());

    fixture.check(head, BTreeMap::new(), true).await;

    assert!(fixture.metas().is_empty(), "the run publishes its own end");
    Ok(())
}

#[tokio::test]
async fn a_task_whose_document_is_missing_is_published_unrunnable() {
    let fixture = Fixture::new();
    let mut head = unclaimed();
    head["agent_exec"]["document"] = json!("MISSING.md");

    fixture.check(head, BTreeMap::new(), true).await;

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

/// A claim publishes only the metadata, which a tree with local changes
/// would sweep in.
#[tokio::test]
async fn a_task_on_a_package_with_local_changes_waits() {
    let fixture = Fixture::new();
    let changes = BTreeMap::from([(
        PathBuf::from("notes.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    )]);

    fixture.check(unclaimed(), changes, true).await;

    assert!(fixture.metas().is_empty(), "nothing is claimed");
    assert!(fixture.runs.lock().unwrap().is_empty());
}

//! A Diverged copy merged onto `latest` by the tick.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;

use super::*;

fn file(key: &str, content: &str) -> quilt::manifest::ManifestRow {
    quilt::manifest::ManifestRow {
        logical_key: PathBuf::from(key),
        physical_key: format!("s3://bucket/acme/demo/{key}?versionId={content}"),
        hash: quilt::object_hash::Sha256Hash::try_from(hex(content).as_str())
            .expect("hex")
            .into(),
        size: content.len() as u64,
        meta: None,
    }
}

fn revision(message: &str, rows: Vec<quilt::manifest::ManifestRow>) -> quilt::manifest::Manifest {
    quilt::manifest::Manifest {
        header: quilt::manifest::ManifestHeader {
            message: Some(message.to_string()),
            ..quilt::manifest::ManifestHeader::default()
        },
        rows,
    }
}

fn ns() -> Namespace {
    ("acme", "demo").into()
}

fn at(hash: &str) -> quilt_uri::ManifestUri {
    quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns(),
        hash: hash.to_string(),
        origin: Some(fixtures::host()),
    }
}

fn change(key: &str) -> quilt::lineage::ChangeSet {
    BTreeMap::from([(
        PathBuf::from(key),
        quilt::lineage::Change::Modified(file(key, "edited")),
    )])
}

/// The revisions around a Diverged copy: the base it shares with `latest`,
/// its own (`ours`, pushed or pending), and `latest`.
struct Copy {
    base: quilt::manifest::Manifest,
    ours: quilt::manifest::Manifest,
    latest: quilt::manifest::Manifest,
    /// Whether `ours` is a local commit not yet pushed.
    pending: bool,
    /// What the folder holds against the copy's revision: at the first walk,
    /// and at the walk before the reset.
    changes: quilt::lineage::ChangeSet,
    changes_before_reset: quilt::lineage::ChangeSet,
    /// What other writers publish, one just before each push onto `latest`.
    writers: Vec<quilt::manifest::Manifest>,
    /// The folders the copy tracks.
    tracked: Vec<&'static str>,
}

impl Copy {
    fn new(
        base: Vec<quilt::manifest::ManifestRow>,
        ours: Vec<quilt::manifest::ManifestRow>,
        latest: Vec<quilt::manifest::ManifestRow>,
    ) -> Self {
        Self {
            base: revision("base", base),
            ours: revision("mine", ours),
            latest: revision("theirs", latest),
            pending: false,
            changes: BTreeMap::new(),
            changes_before_reset: BTreeMap::new(),
            writers: Vec::new(),
            tracked: Vec::new(),
        }
    }
}

/// The remote the merge reads and publishes to, and what the tick did.
#[derive(Clone)]
struct Remote {
    revisions: BTreeMap<String, quilt::manifest::Manifest>,
    /// The hash `latest` names.
    latest: String,
    writers: VecDeque<quilt::manifest::Manifest>,
    /// What another writer publishes between the merge's push and the reset.
    late_writer: Option<quilt::manifest::Manifest>,
    /// Whether the tray counted the reset as an apply, read as it ran.
    aggregator: Option<Arc<crate::engine::autopull::status::SyncTrayAggregator>>,
    applying_at_reset: Option<bool>,
    pushes: usize,
    events: Vec<String>,
}

impl Remote {
    fn latest(&self) -> &quilt::manifest::Manifest {
        &self.revisions[&self.latest]
    }
}

/// The remote as the tick left it.
fn seen(remote: &Mutex<Remote>) -> Remote {
    remote.lock().unwrap().clone()
}

const NOTHING: [&str; 0] = [];

/// A model over `copy`: `latest` is `h1`, the copy's own revision `ours`, its
/// base `h0`.
fn model_of(copy: Copy) -> (MockQuiltModel, Arc<Mutex<Remote>>) {
    let remote = Arc::new(Mutex::new(Remote {
        revisions: BTreeMap::from([
            ("h0".to_string(), copy.base),
            ("ours".to_string(), copy.ours),
            ("h1".to_string(), copy.latest),
        ]),
        latest: "h1".to_string(),
        writers: copy.writers.into(),
        late_writer: None,
        aggregator: None,
        applying_at_reset: None,
        pushes: 0,
        events: Vec::new(),
    }));
    let lineage = Arc::new(Mutex::new(quilt::lineage::PackageLineage {
        commit: copy.pending.then(|| quilt::lineage::CommitState {
            hash: "ours".to_string(),
            ..quilt::lineage::CommitState::default()
        }),
        remote_uri: Some(at(if copy.pending { "h0" } else { "ours" })),
        base_hash: "h0".to_string(),
        latest_hash: "h1".to_string(),
        paths: copy
            .tracked
            .iter()
            .map(|key| (PathBuf::from(key), quilt::lineage::PathState::default()))
            .collect(),
        ..quilt::lineage::PackageLineage::default()
    }));

    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            quilt::LocalDomain::new(PathBuf::new()).create_installed_package(ns()),
        ])
    });
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new()).create_installed_package(ns()),
        ))
    });
    model
        .expect_try_lock_package()
        .returning(|p| Ok(Some(p.namespace.clone())));
    let state = Arc::clone(&lineage);
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(state.lock().unwrap().clone()));
    let walks = Mutex::new(VecDeque::from([copy.changes, copy.changes_before_reset]));
    model.expect_locked_package_status().returning(move |_, _| {
        let changes = walks.lock().unwrap().pop_front().unwrap_or_default();
        Ok(quilt::lineage::InstalledPackageStatus::new(
            UpstreamState::Diverged,
            changes,
        ))
    });
    model.expect_resolve_workflow().returning(|_, _| Ok(None));
    let (state, events) = (Arc::clone(&lineage), Arc::clone(&remote));
    model
        .expect_locked_package_publish()
        .returning(move |_, _, _, _, _, _| {
            events.lock().unwrap().events.push("publish".to_string());
            let mut lineage = state.lock().unwrap();
            lineage.commit = None;
            lineage.remote_uri = Some(at("ours"));
            Ok(quilt::PublishOutcome::PushedOnly(quilt::PushOutcome {
                manifest_uri: at("ours"),
                certified_latest: false,
            }))
        });
    expect_remote(&mut model, &remote);
    (model, remote)
}

/// The remote's reads and writes, and the reset that follows them.
fn expect_remote(model: &mut MockQuiltModel, remote: &Arc<Mutex<Remote>>) {
    let state = Arc::clone(remote);
    model
        .expect_browse_remote_manifest()
        .returning(move |uri| Ok(state.lock().unwrap().revisions[&uri.hash].clone()));
    let state = Arc::clone(remote);
    model
        .expect_resolve_manifest_uri()
        .returning(move |_| Ok(at(&state.lock().unwrap().latest)));
    let state = Arc::clone(remote);
    model.expect_push_onto().returning(move |parent, manifest| {
        let mut remote = state.lock().unwrap();
        remote.events.push(format!("push onto {}", parent.hash));
        remote.pushes += 1;
        let n = remote.pushes;
        if let Some(theirs) = remote.writers.pop_front() {
            remote.revisions.insert(format!("w{n}"), theirs);
            remote.latest = format!("w{n}");
        }
        let hash = format!("r{n}");
        remote.revisions.insert(hash.clone(), manifest);
        let certified_latest = remote.latest == parent.hash;
        if certified_latest {
            remote.latest.clone_from(&hash);
        }
        Ok(quilt::PushOutcome {
            manifest_uri: at(&hash),
            certified_latest,
        })
    });
    let state = Arc::clone(remote);
    model
        .expect_locked_package_reset_local()
        .returning(move |_| {
            let mut remote = state.lock().unwrap();
            remote.events.push("reset".to_string());
            remote.applying_at_reset = remote
                .aggregator
                .as_ref()
                .map(|aggregator| aggregator.apply_in_progress());
            if let Some(theirs) = remote.late_writer.take() {
                remote.revisions.insert("late".to_string(), theirs);
                remote.latest = "late".to_string();
            }
            Ok(at(&remote.latest))
        });
}

async fn tick(model: &MockQuiltModel) -> (WatcherInner, Arc<RecordingReporter>) {
    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        reporter: reporter.clone(),
        last_change: LastChange::changed_for_test(&ns(), Duration::from_hours(1)).await,
        ..make_inner(enabled())
    };
    run_once(model, &RoleCache::default(), &inner)
        .await
        .expect("ticked");
    (inner, reporter)
}

fn keys(manifest: &quilt::manifest::Manifest) -> Vec<String> {
    manifest
        .rows
        .iter()
        .map(|row| row.logical_key.display().to_string())
        .collect()
}

#[tokio::test]
async fn changes_to_different_files_land_on_latest_together_and_the_copy_resumes() {
    let (model, remote) = model_of(Copy {
        tracked: vec!["a.csv"],
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });

    let (inner, reporter) = tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.latest, "r1");
    assert_eq!(
        remote.latest().rows,
        [file("a.csv", "mine"), file("b.csv", "theirs")]
    );
    assert_eq!(remote.events, ["push onto h1", "reset"]);
    assert!(inner.paused.read().await.is_empty(), "not paused");
    assert_eq!(
        *reporter.published.lock().unwrap(),
        [(ns(), "mine".to_string())],
        "reported as a publish"
    );
    let pulled = reporter.pulled.lock().unwrap();
    assert_eq!(pulled.len(), 1, "and as a pull");
    assert_eq!(pulled[0].1.groups[0].items, ["b.csv"]);
}

#[tokio::test]
async fn the_merged_revision_carries_this_copys_message_under_latests_header() {
    let mut copy = Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    );
    copy.latest.header.workflow = Some(quilt::manifest::Workflow {
        config: "s3://bucket/.quilt/workflows/config.yml".parse().unwrap(),
        id: None,
    });
    let workflow = copy.latest.header.workflow.clone();
    let (model, remote) = model_of(copy);

    tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.latest().header.message.as_deref(), Some("mine"));
    assert_eq!(remote.latest().header.workflow, workflow);
}

/// Two local revisions: the first added `one.csv`, the newest `two.csv`.
#[tokio::test]
async fn unpublished_local_revisions_are_pushed_and_land_as_one_revision() {
    let (model, remote) = model_of(Copy {
        pending: true,
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![
                file("a.csv", "1"),
                file("one.csv", "1"),
                file("two.csv", "2"),
            ],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });

    tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(
        remote.events,
        ["publish", "push onto h1", "reset"],
        "the local commit is pushed first, then one merged revision"
    );
    assert_eq!(
        keys(remote.latest()),
        ["a.csv", "b.csv", "one.csv", "two.csv"]
    );
}

#[tokio::test]
async fn edits_are_committed_before_the_merge_once_their_wait_is_over() {
    let (model, remote) = model_of(Copy {
        pending: true,
        changes: change("a.csv"),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "edited")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });

    tick(&model).await;

    assert_eq!(seen(&remote).events, ["publish", "push onto h1", "reset"]);
}

#[tokio::test]
async fn edits_still_in_their_wait_are_neither_committed_nor_merged() {
    let (model, remote) = model_of(Copy {
        changes: change("a.csv"),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "1")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });
    let inner = WatcherInner {
        last_change: LastChange::changed_for_test(&ns(), Duration::ZERO).await,
        ..make_inner(enabled())
    };

    run_once(&model, &RoleCache::default(), &inner)
        .await
        .expect("ticked");

    assert_eq!(seen(&remote).events, NOTHING);
    assert!(inner.paused.read().await.is_empty(), "waiting, not paused");
}

#[tokio::test]
async fn a_file_both_sides_changed_differently_stays_diverged_and_publishes_nothing() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "theirs")],
    ));

    let (inner, _) = tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.events, NOTHING);
    assert_eq!(remote.latest, "h1");
    assert_eq!(
        inner.paused.read().await.get(&ns()),
        Some(&PausedReason::Diverged)
    );
}

#[tokio::test]
async fn metadata_both_sides_changed_differently_stays_diverged() {
    let mut copy = Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1")],
    );
    copy.ours.header.user_meta = Some(json!({"owner": "me"}));
    copy.latest.header.user_meta = Some(json!({"owner": "them"}));
    let (model, remote) = model_of(copy);

    let (inner, _) = tick(&model).await;

    assert_eq!(seen(&remote).events, NOTHING);
    assert_eq!(
        inner.paused.read().await.get(&ns()),
        Some(&PausedReason::Diverged)
    );
}

#[tokio::test]
async fn a_writer_who_publishes_during_the_merge_keeps_latest_and_the_merge_is_redone_onto_it() {
    let theirs = revision(
        "another",
        vec![file("a.csv", "1"), file("c.csv", "another")],
    );
    let (model, remote) = model_of(Copy {
        writers: vec![theirs],
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1")],
        )
    });

    tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.events, ["push onto h1", "push onto w1", "reset"]);
    assert_eq!(
        remote.latest().rows,
        [file("a.csv", "mine"), file("c.csv", "another")]
    );
}

#[tokio::test]
async fn a_latest_that_keeps_moving_pauses_the_copy_after_three_attempts() {
    let others =
        ["x", "y", "z"].map(|name| revision(name, vec![file("a.csv", "1"), file(name, name)]));
    let (model, remote) = model_of(Copy {
        writers: others.into(),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1")],
        )
    });

    let (inner, _) = tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(
        remote.events,
        ["push onto h1", "push onto w1", "push onto w2"]
    );
    assert_eq!(remote.latest, "w3", "the last writer keeps latest");
    assert_eq!(
        inner.paused.read().await.get(&ns()),
        Some(&PausedReason::Diverged)
    );
}

#[tokio::test]
async fn an_edit_during_the_merge_keeps_the_copy_diverged_with_latest_holding_its_change() {
    let (model, remote) = model_of(Copy {
        changes_before_reset: change("a.csv"),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });

    let (inner, _) = tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.events, ["push onto h1"], "no reset over the edit");
    assert_eq!(remote.latest().rows[0], file("a.csv", "mine"));
    assert!(
        inner.paused.read().await.is_empty(),
        "the next tick retries"
    );
}

/// The tick after an edit kept the copy at its own revision: `latest` already
/// holds that revision's change.
#[tokio::test]
async fn a_copy_whose_change_latest_already_holds_resets_without_publishing() {
    let (model, remote) = model_of(Copy {
        latest: revision("mine", vec![file("a.csv", "mine"), file("b.csv", "theirs")]),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![],
        )
    });

    let (inner, reporter) = tick(&model).await;

    assert_eq!(seen(&remote).events, ["reset"]);
    assert!(reporter.published.lock().unwrap().is_empty());
    assert!(inner.paused.read().await.is_empty());
}

#[tokio::test]
async fn a_package_a_task_run_holds_is_not_merged() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    let inner = WatcherInner {
        last_change: LastChange::changed_for_test(&ns(), Duration::from_hours(1)).await,
        ..make_inner(enabled())
    };
    let _run = inner.tasks.hold(&ns());

    run_once(&model, &RoleCache::default(), &inner)
        .await
        .expect("ticked");

    assert_eq!(seen(&remote).events, NOTHING);
}

#[tokio::test]
async fn with_pulling_off_a_diverged_copy_pauses_as_before() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    let mut settings = enabled();
    settings.pull.enabled = false;
    let inner = WatcherInner {
        last_change: LastChange::changed_for_test(&ns(), Duration::from_hours(1)).await,
        ..make_inner(settings)
    };

    run_once(&model, &RoleCache::default(), &inner)
        .await
        .expect("ticked");

    assert_eq!(seen(&remote).events, NOTHING);
    assert_eq!(
        inner.paused.read().await.get(&ns()),
        Some(&PausedReason::Diverged)
    );
}

#[tokio::test]
async fn the_reset_after_a_merge_counts_as_writing_the_folder() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    let aggregator = test_aggregator();
    remote.lock().unwrap().aggregator = Some(Arc::clone(&aggregator));
    let inner = WatcherInner {
        last_change: LastChange::changed_for_test(&ns(), Duration::from_hours(1)).await,
        ..inner_with(aggregator)
    };

    run_once(&model, &RoleCache::default(), &inner)
        .await
        .expect("ticked");

    assert_eq!(seen(&remote).applying_at_reset, Some(true));
}

/// Another writer published between the merge's push and the reset, so the
/// reset wrote a revision the merge did not make.
#[tokio::test]
async fn a_reset_that_lands_past_the_merged_revision_reports_no_files() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    remote.lock().unwrap().late_writer = Some(revision(
        "late",
        vec![file("a.csv", "mine"), file("c.csv", "late")],
    ));

    let (_, reporter) = tick(&model).await;

    assert_eq!(seen(&remote).events, ["push onto h1", "reset"]);
    assert_eq!(reporter.pulled.lock().unwrap().len(), 0);
}

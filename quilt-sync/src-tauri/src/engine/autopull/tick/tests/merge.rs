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
    /// What the folder holds against the copy's revision, one entry per walk
    /// in order; clean once they run out.
    walks: Vec<quilt::lineage::ChangeSet>,
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
            walks: Vec::new(),
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
    /// What another writer publishes just after the merge's push lands.
    late_writer: Option<quilt::manifest::Manifest>,
    /// Where a local file refuses the move, as a pull refuses.
    conflict_at: Option<&'static str>,
    /// Whether the tray counted the move as an apply, read as it ran.
    aggregator: Option<Arc<crate::engine::autopull::status::SyncTrayAggregator>>,
    applying_at_move: Option<bool>,
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
        conflict_at: None,
        aggregator: None,
        applying_at_move: None,
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
    let walks = Mutex::new(VecDeque::from(copy.walks));
    let state = Arc::clone(&lineage);
    model.expect_locked_package_status().returning(move |_, _| {
        let changes = walks.lock().unwrap().pop_front().unwrap_or_default();
        Ok(quilt::lineage::InstalledPackageStatus::new(
            state.lock().unwrap().clone().into(),
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
    expect_move(&mut model, &lineage, &remote);
    (model, remote)
}

/// The move after the merge, which moves the copy's lineage to its target.
fn expect_move(
    model: &mut MockQuiltModel,
    lineage: &Arc<Mutex<quilt::lineage::PackageLineage>>,
    remote: &Arc<Mutex<Remote>>,
) {
    let (state, moves) = (Arc::clone(lineage), Arc::clone(remote));
    model
        .expect_locked_package_move_to()
        .returning(move |_, target, _| {
            let mut remote = moves.lock().unwrap();
            remote.events.push(format!("move to {}", target.hash));
            remote.applying_at_move = remote
                .aggregator
                .as_ref()
                .map(|aggregator| aggregator.apply_in_progress());
            if let Some(path) = remote.conflict_at {
                return Err(
                    quilt::Error::PackageOp(quilt::PackageOpError::PullConflict(vec![
                        PathBuf::from(path),
                    ]))
                    .into(),
                );
            }
            let mut lineage = state.lock().unwrap();
            lineage.remote_uri = Some(target.clone());
            lineage.base_hash.clone_from(&target.hash);
            lineage.latest_hash.clone_from(&target.hash);
            Ok(quilt::flow::Pulled::Applied(moved(
                &remote.revisions["ours"],
                &remote.revisions[&target.hash],
                target,
            )))
        });
}

/// What a move from `ours` to `target` writes, as the pull it runs reports it.
fn moved(
    ours: &quilt::manifest::Manifest,
    target: &quilt::manifest::Manifest,
    uri: &quilt_uri::ManifestUri,
) -> quilt::flow::PullReport {
    let mut report = quilt::flow::PullReport {
        manifest_uri: uri.clone(),
        added: Vec::new(),
        added_not_fetched: Vec::new(),
        updated: Vec::new(),
        removed: Vec::new(),
        message: target.header.message.clone(),
    };
    for row in &target.rows {
        match ours
            .rows
            .iter()
            .find(|was| was.logical_key == row.logical_key)
        {
            None => report.added_not_fetched.push(row.logical_key.clone()),
            Some(was) if was.hash != row.hash => report.updated.push(row.logical_key.clone()),
            Some(_) => {}
        }
    }
    report
}

/// The remote's reads and writes.
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
            if let Some(theirs) = remote.late_writer.take() {
                remote.revisions.insert("late".to_string(), theirs);
                remote.latest = "late".to_string();
            }
        }
        Ok(quilt::PushOutcome {
            manifest_uri: at(&hash),
            certified_latest,
        })
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
    assert_eq!(remote.events, ["push onto h1", "move to r1"]);
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
        ["publish", "push onto h1", "move to r1"],
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
        walks: vec![change("a.csv")],
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "edited")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });

    tick(&model).await;

    assert_eq!(
        seen(&remote).events,
        ["publish", "push onto h1", "move to r1"]
    );
}

#[tokio::test]
async fn edits_still_in_their_wait_are_neither_committed_nor_merged() {
    let (model, remote) = model_of(Copy {
        walks: vec![change("a.csv")],
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
    assert_eq!(
        remote.events,
        ["push onto h1", "push onto w1", "move to r2"]
    );
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

/// `a.csv` is edited again while the merge publishes it. The move keeps the
/// edit on top of the merged revision, so the next tick publishes it as any
/// change rather than merging it against the copy's own.
#[tokio::test]
async fn an_edit_during_the_merge_to_a_path_it_published_is_published_next_tick() {
    let (model, remote) = model_of(Copy {
        walks: vec![BTreeMap::new(), change("a.csv"), change("a.csv")],
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });
    let mut settings = enabled();
    settings.push.idle_timeout_secs = 0;
    let inner = WatcherInner {
        last_change: LastChange::changed_for_test(&ns(), Duration::from_hours(1)).await,
        ..make_inner(settings)
    };

    for _ in 0..2 {
        run_once(&model, &RoleCache::default(), &inner)
            .await
            .expect("ticked");
    }

    assert_eq!(
        seen(&remote).events,
        ["push onto h1", "move to r1", "publish"],
        "published as an ordinary change, not merged again"
    );
    assert!(inner.paused.read().await.is_empty());
}

/// An edit to `b.csv`, which the other side changed, refuses the move as it
/// refuses a pull. The merged revision stays published, and the copy stays
/// Diverged at its own revision for the next tick.
#[tokio::test]
async fn an_edit_during_the_merge_to_a_path_the_other_side_changed_keeps_the_copy_diverged() {
    let (model, remote) = model_of(Copy {
        walks: vec![BTreeMap::new(), change("b.csv")],
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![file("a.csv", "1"), file("b.csv", "theirs")],
        )
    });
    remote.lock().unwrap().conflict_at = Some("b.csv");

    let (inner, reporter) = tick(&model).await;

    let remote = seen(&remote);
    assert_eq!(remote.events, ["push onto h1", "move to r1"]);
    assert_eq!(remote.latest, "r1");
    assert!(
        inner.paused.read().await.is_empty(),
        "the next tick retries"
    );
    assert_eq!(reporter.published.lock().unwrap().len(), 1);
    assert!(
        reporter.pulled.lock().unwrap().is_empty(),
        "nothing written"
    );
}

/// A local file the walk does not list, such as an ignored one, where the
/// other side added `b.csv`: no commit will change the next merge, so the
/// copy pauses as such a pull pauses.
#[tokio::test]
async fn a_move_refused_by_a_file_that_is_not_an_edit_pauses_as_a_pull_conflict() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    remote.lock().unwrap().conflict_at = Some("b.csv");

    let (inner, _) = tick(&model).await;

    assert_eq!(
        inner.paused.read().await.get(&ns()),
        Some(&PausedReason::pull_conflict(&[PathBuf::from("b.csv")]))
    );
}

/// The copy moves to the revision the merge published, and reports what that
/// brought, though another writer has published since.
#[tokio::test]
async fn the_copy_moves_to_the_merged_revision_not_one_published_after_it() {
    let (model, remote) = model_of(Copy::new(
        vec![file("a.csv", "1")],
        vec![file("a.csv", "mine")],
        vec![file("a.csv", "1"), file("b.csv", "theirs")],
    ));
    remote.lock().unwrap().late_writer = Some(revision(
        "late",
        vec![
            file("a.csv", "mine"),
            file("b.csv", "theirs"),
            file("c.csv", "late"),
        ],
    ));

    let (_, reporter) = tick(&model).await;

    assert_eq!(seen(&remote).events, ["push onto h1", "move to r1"]);
    let pulled = reporter.pulled.lock().unwrap();
    assert_eq!(pulled[0].1.groups[0].items, ["b.csv"]);
}

/// The tick after an edit kept the copy at its own revision: `latest` already
/// holds that revision's change, under the message of a later revision.
#[tokio::test]
async fn a_copy_whose_change_latest_already_holds_moves_to_it_without_publishing() {
    let (model, remote) = model_of(Copy {
        latest: revision(
            "a later revision",
            vec![file("a.csv", "mine"), file("b.csv", "theirs")],
        ),
        ..Copy::new(
            vec![file("a.csv", "1")],
            vec![file("a.csv", "mine")],
            vec![],
        )
    });

    let (inner, reporter) = tick(&model).await;

    assert_eq!(seen(&remote).events, ["move to h1"]);
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
async fn the_move_after_a_merge_counts_as_writing_the_folder() {
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

    assert_eq!(seen(&remote).applying_at_move, Some(true));
}

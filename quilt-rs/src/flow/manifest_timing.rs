//! How long the flows that look up each file in a manifest take on a big
//! package. A measurement, run on demand and not in CI, in release mode:
//!
//! ```text
//! MANIFEST_TIMING_ROWS=10000,100000 \
//!   cargo test --release -p quilt-rs --lib manifest_timing -- --ignored --nocapture
//! ```
//!
//! Every file is one small object, already in the local cache, so the times
//! are the flows' own work plus reading and writing small files.

use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

use super::pull_outcome::remote_delta;
use crate::Res;
use crate::checksum::calculate_hash;
use crate::flow;
use crate::io::remote::HostConfig;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::lineage::Change;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;

fn key(file: usize) -> String {
    format!("data/plate-{:04}/well-{file:07}.tsv", file / 96)
}

fn row_counts() -> Vec<usize> {
    std::env::var("MANIFEST_TIMING_ROWS")
        .unwrap_or_else(|_| "10000".to_string())
        .split(',')
        .map(|n| n.trim().parse().expect("MANIFEST_TIMING_ROWS: numbers"))
        .collect()
}

/// Writes the fixture's files without the storage layer's syncing, which
/// would take most of the run.
fn write(path: &std::path::Path, text: &str) -> Res {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    std::fs::write(path, text)?;
    Ok(())
}

fn secs(duration: Duration) -> String {
    format!("{:.2} s", duration.as_secs_f64())
}

async fn measure(rows: usize) -> Res<Vec<(&'static str, Duration)>> {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let paths = DomainPaths::new(dir.path().join("domain"));
    let home = dir.path().join("home");
    let source = dir.path().join("source");
    let host_config = HostConfig::default();
    let namespace: Namespace = ("timing", "big").into();

    // Each file's bytes are its key, so every object differs. They go to the
    // object cache, so installing copies them without a fetch.
    let mut manifest = Manifest::default();
    for file in 0..rows {
        let key = key(file);
        let at = source.join(&key);
        write(&at, &key)?;
        let row = ManifestRow {
            physical_key: format!("s3://bucket/{key}"),
            ..calculate_hash(&storage, &at, std::path::Path::new(&key), &host_config).await?
        };
        write(&paths.object(row.hash.digest()), &key)?;
        manifest.rows.push(row);
    }
    let lineage = PackageLineage {
        remote_uri: Some(ManifestUri {
            bucket: "bucket".to_string(),
            namespace: namespace.clone(),
            hash: "1111".to_string(),
            origin: None,
        }),
        base_hash: "1111".to_string(),
        latest_hash: "1111".to_string(),
        ..PackageLineage::default()
    };
    let keys: Vec<PathBuf> = (0..rows).map(|file| PathBuf::from(key(file))).collect();
    let entries: Vec<&PathBuf> = keys.iter().collect();
    let remote = MockRemote::default();

    let mut times = Vec::new();

    let start = Instant::now();
    let (lineage, _) = flow::install_paths(
        lineage,
        &mut manifest,
        &paths,
        home.clone(),
        namespace.clone(),
        &storage,
        &remote,
        &entries,
    )
    .await?;
    times.push(("install_paths", start.elapsed()));
    assert_eq!(lineage.paths.len(), rows);

    let start = Instant::now();
    let (lineage, status) = super::status::create_status(
        lineage,
        &storage,
        &paths,
        &manifest,
        &home,
        host_config.clone(),
    )
    .await?;
    times.push(("status", start.elapsed()));
    assert!(status.changes.is_empty());

    // A remote revision that changes every tenth file and adds as many.
    let mut latest = manifest.clone();
    let mut changed = 0;
    for row in latest.rows.iter_mut().step_by(10) {
        row.hash = manifest.rows[rows - 1].hash.clone();
        changed += 1;
    }
    for file in rows..rows + rows / 10 {
        latest.rows.push(ManifestRow {
            logical_key: crate::LogicalKey::new(key(file))?,
            ..manifest.rows[0].clone()
        });
    }
    let start = Instant::now();
    let delta = remote_delta(&manifest, &latest);
    times.push(("pull diff", start.elapsed()));
    assert_eq!(delta.len(), changed + rows / 10);

    // A commit that adds a tenth as many files again.
    for file in rows..rows + rows / 10 {
        let key = key(file);
        write(&home.join(&key), &key)?;
    }
    let (lineage, status) = super::status::create_status(
        lineage,
        &storage,
        &paths,
        &manifest,
        &home,
        host_config.clone(),
    )
    .await?;
    assert_eq!(status.changes.len(), rows / 10);
    assert!(
        status
            .changes
            .values()
            .all(|change| matches!(change, Change::Added(_)))
    );
    let status = flow::rehash_changes(&storage, &manifest, &home, status, &host_config).await?;
    let start = Instant::now();
    let (lineage, _) = flow::commit(
        lineage,
        &mut manifest,
        &paths,
        &storage,
        &remote,
        flow::WorkflowCheck::Gate { host: None },
        home,
        status,
        namespace,
        String::from("timing"),
        flow::UserMeta::Keep,
        None,
    )
    .await?;
    times.push(("commit", start.elapsed()));
    assert!(lineage.commit.is_some());

    Ok(times)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement: run on demand, in release mode"]
async fn manifest_timing() -> Res {
    for rows in row_counts() {
        for (operation, took) in measure(rows).await? {
            println!("{rows:>8} rows  {operation:<14} {}", secs(took));
        }
    }
    Ok(())
}

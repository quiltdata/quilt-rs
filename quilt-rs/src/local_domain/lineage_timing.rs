//! How long the reads behind `quilt list` and `quilt status` take on each
//! lineage store. A measurement, run on demand and not in CI, in release
//! mode:
//!
//! ```text
//! cargo test --release -p quilt-rs --lib lineage_timing -- --ignored --nocapture
//! ```
//!
//! It fails where refs' median is above `data.json`'s upper quartile: the
//! spec allows `list` and `status` no slowdown, and gives no number for
//! noise.

use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

use super::*;
use crate::fixtures::create_multihash;
use crate::fixtures::objects::ZERO_HASH_B64;
use crate::io::manifest::tag_latest;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineageStore;
use crate::lineage::PackageLineage;
use crate::lineage::WorkingCopy;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::paths::DomainPaths;

const RUNS: usize = 11;

const BUCKET: &str = "bucket";
const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
/// Not `BASE`, so on refs each package has a tip cache to read too.
const LATEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// `data.json` first: refs is judged against its timings.
const STORES: [(&str, LineageStore); 2] = [
    ("data.json", LineageStore::DataJson),
    ("refs", LineageStore::Refs),
];

struct Shape {
    name: &'static str,
    /// Each package's tracked paths; the first is the largest, the one
    /// `status` reads.
    packages: Vec<usize>,
}

/// The `data.json` of a real machine: 18 packages, 3,532 paths, 747 KB, the
/// largest package 1,987 paths.
fn real() -> Shape {
    let mut packages = vec![1987];
    packages.extend([91; 15]);
    packages.extend([90; 2]);
    Shape {
        name: "real",
        packages,
    }
}

fn stress() -> Shape {
    Shape {
        name: "stress",
        packages: vec![2000; 200],
    }
}

fn key(package: usize, file: usize) -> String {
    format!(
        "data/runs-{package:03}/plate-{:03}/well-{file:05}.tsv",
        file / 96
    )
}

fn remote(namespace: &Namespace, hash: &str) -> ManifestUri {
    ManifestUri {
        origin: None,
        bucket: BUCKET.to_string(),
        namespace: namespace.clone(),
        hash: hash.to_string(),
    }
}

fn lineage(namespace: &Namespace, package: usize, files: usize) -> Res<PackageLineage> {
    let state = PathState {
        timestamp: chrono::Utc::now(),
        hash: Some(create_multihash(ZERO_HASH_B64)?),
    };
    let paths = (0..files)
        .map(|file| (PathBuf::from(key(package, file)), state.clone()))
        .collect();
    Ok(PackageLineage {
        paths,
        ..PackageLineage::from_remote(remote(namespace, BASE), LATEST.to_string())
    })
}

fn manifest(package: usize, files: usize) -> Res<Manifest> {
    let mut rows = Vec::with_capacity(files);
    for file in 0..files {
        let key = key(package, file);
        rows.push(ManifestRow {
            physical_key: format!("s3://{BUCKET}/{key}"),
            logical_key: key.try_into()?,
            hash: create_multihash(ZERO_HASH_B64)?.try_into()?,
            size: 0,
            meta: None,
            meta_numbers: RawNumbers::default(),
        });
    }
    Ok(Manifest {
        rows,
        ..Manifest::default()
    })
}

struct Fixture {
    domain: LocalDomain<LocalStorage, MockRemote>,
    /// The package `status` reads.
    largest: Namespace,
    /// What the store holds on disk.
    bytes: u64,
    _dir: tempfile::TempDir,
}

/// A domain on `store` with `shape`'s packages, each installed from
/// `BUCKET`, whose `latest` is `LATEST`. No working folder exists, so
/// `status` stops before its walk of the files, which reads no lineage.
async fn build(store: LineageStore, shape: &Shape) -> Res<Fixture> {
    let (paths, dir) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let remote_store = MockRemote::default();
    let mut packages = Vec::new();
    for (package, &files) in shape.packages.iter().enumerate() {
        // An owner each: on refs, the most folders a listing can read.
        let namespace = Namespace::from((format!("owner-{package:03}").as_str(), "package"));
        tag_latest(&remote_store, &remote(&namespace, LATEST)).await?;
        packages.push((namespace.clone(), lineage(&namespace, package, files)?));
    }
    let largest = packages[0].0.clone();
    let installed = paths.installed_manifest(&largest, BASE);
    storage
        .create_dir_all(installed.parent().expect("a folder"))
        .await?;
    storage
        .write_byte_stream(installed, (&manifest(0, shape.packages[0])?).into())
        .await?;
    let io =
        DomainLineageIo::write_fixture(store, &storage, &paths, dir.path().join("home"), packages)
            .await?;
    let bytes = match store {
        LineageStore::DataJson => std::fs::metadata(paths.lineage())?.len(),
        LineageStore::Refs => {
            bytes_under(&paths.refs_dir())? + std::fs::metadata(paths.domain_config())?.len()
        }
    };
    Ok(Fixture {
        domain: LocalDomain::with_parts(paths, io, storage, Arc::new(remote_store)),
        largest,
        bytes,
        _dir: dir,
    })
}

fn bytes_under(dir: &Path) -> Res<u64> {
    let mut bytes = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        bytes += if entry.file_type()?.is_dir() {
            bytes_under(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(bytes)
}

/// The median, the upper quartile and the worst of `RUNS` timings of `op`,
/// after one untimed run.
async fn time<T, F: Future<Output = Res<T>>>(
    mut op: impl FnMut() -> F,
) -> Res<(Duration, Duration, Duration)> {
    op().await?;
    let mut runs = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let start = Instant::now();
        op().await?;
        runs.push(start.elapsed());
    }
    runs.sort();
    Ok((runs[RUNS / 2], runs[RUNS * 3 / 4], runs[RUNS - 1]))
}

/// What the quilt CLI runs before every command that needs a home: a sweep,
/// then two reads of the home.
async fn start(domain: &LocalDomain<LocalStorage, MockRemote>) -> Res {
    domain.sweep_leftovers().await?;
    domain.get_home().await?;
    domain.get_home().await?;
    Ok(())
}

/// What `quilt list` runs: every package's lineage.
async fn list(
    domain: &LocalDomain<LocalStorage, MockRemote>,
) -> Res<BTreeMap<Namespace, Res<PackageLineage>>> {
    start(domain).await?;
    domain.read_packages().await
}

/// What `quilt status --namespace` runs.
async fn status(
    domain: &LocalDomain<LocalStorage, MockRemote>,
    namespace: &Namespace,
) -> Res<InstalledPackageStatus> {
    start(domain).await?;
    domain
        .get_installed_package(namespace)
        .await?
        .expect("installed")
        .status(None)
        .await
}

fn ms(duration: Duration) -> String {
    format!("{:.3} ms", duration.as_secs_f64() * 1e3)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement: run on demand, in release mode"]
async fn lineage_timing() -> Res {
    let mut table = vec![format!(
        "{:<8} {:<10} {:<10} {:>12} {:>12} {:>12}",
        "shape", "operation", "store", "median", "quartile", "worst"
    )];
    let mut slower = Vec::new();
    for shape in [real(), stress()] {
        let paths: usize = shape.packages.iter().sum();
        let mut quartile_on_data_json = BTreeMap::new();
        for (store_name, store) in STORES {
            let Fixture {
                domain,
                largest,
                bytes,
                _dir,
            } = build(store, &shape).await?;
            println!(
                "{} on {store_name}: {} packages, {paths} paths, {} KB",
                shape.name,
                shape.packages.len(),
                bytes / 1000,
            );
            let listed = list(&domain).await?;
            assert_eq!(listed.len(), shape.packages.len());
            let mut listed_paths = 0;
            for lineage in listed.into_values() {
                let lineage = lineage?;
                assert_eq!(lineage.latest_hash, LATEST);
                listed_paths += lineage.paths.len();
            }
            assert_eq!(listed_paths, paths);
            let status_of_largest = status(&domain, &largest).await?;
            assert!(matches!(
                status_of_largest.working_copy,
                WorkingCopy::Unavailable(_)
            ));

            for (operation, (median, quartile, worst)) in [
                ("list", time(|| list(&domain)).await?),
                ("status", time(|| status(&domain, &largest)).await?),
            ] {
                table.push(format!(
                    "{:<8} {operation:<10} {store_name:<10} {:>12} {:>12} {:>12}",
                    shape.name,
                    ms(median),
                    ms(quartile),
                    ms(worst),
                ));
                if store == LineageStore::DataJson {
                    quartile_on_data_json.insert(operation, quartile);
                } else if median > quartile_on_data_json[operation] {
                    slower.push(format!("{} {operation}", shape.name));
                }
            }
        }
    }
    println!("{}", table.join("\n"));
    assert!(
        slower.is_empty(),
        "refs slower than data.json: {}",
        slower.join(", ")
    );
    Ok(())
}

//! Two clients publishing the same package to one bucket: neither may move
//! `latest` over a revision it has not seen.
//!
//! A push by any client leaves three objects: the manifest, a timestamp
//! pointer and `latest`. Nothing in them says who wrote them. Here the other
//! client is either a second domain on the same mock bucket, or the objects
//! quilt3 wrote in `fixtures/quilt3-bucket/`, loaded into the bucket as they
//! are.

use std::path::Path;
use std::str::FromStr;

use super::*;

use super::older_revision_tests::BUCKET;
use super::older_revision_tests::publish_manifest;
use super::older_revision_tests::publish_row;

use crate::io::remote::mocks::MockRemote;
use crate::lineage::UpstreamState;
use crate::paths::DomainPaths;
use quilt_uri::S3Uri;

type Domain = crate::LocalDomain<LocalStorage, MockRemote>;
type Package = InstalledPackage<LocalStorage, MockRemote>;

/// One client's machine: a domain with its own home, on the shared bucket.
struct Client {
    domain: Domain,
    _dirs: [tempfile::TempDir; 2],
}

impl Client {
    async fn new(bucket: &Arc<MockRemote>) -> Res<Self> {
        let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
        let home_dir = tempfile::TempDir::new()?;
        let domain = crate::LocalDomain::with_parts(paths, LocalStorage::new(), Arc::clone(bucket));
        domain.set_home(home_dir.path()).await?;
        Ok(Self {
            domain,
            _dirs: [paths_dir, home_dir],
        })
    }

    /// Installs `hash` of `namespace` in `bucket_name` with every file downloaded.
    async fn install(&self, bucket_name: &str, namespace: &Namespace, hash: &str) -> Res<Package> {
        let package = self
            .domain
            .install_package(&ManifestUri {
                bucket: bucket_name.to_string(),
                namespace: namespace.clone(),
                hash: hash.to_string(),
                origin: None,
            })
            .await?;
        let paths: Vec<PathBuf> = package
            .manifest()
            .await?
            .rows
            .iter()
            .map(|row| PathBuf::from(row.logical_key.clone()))
            .collect();
        package.install_paths(&paths).await?;
        Ok(package)
    }
}

async fn write_file(package: &Package, name: &str, body: &str) -> Res {
    let path = package.package_home().await?.join(name);
    tokio::fs::write(path, body).await?;
    Ok(())
}

async fn read_file(package: &Package, name: &str) -> Res<String> {
    let path = package.package_home().await?.join(name);
    Ok(tokio::fs::read_to_string(path).await?)
}

async fn commit(package: &Package, message: &str) -> Res<String> {
    let committed = package
        .commit(message.to_string(), UserMeta::Keep, None, None)
        .await?;
    Ok(committed.hash().expect("a change to commit").to_string())
}

async fn push(package: &Package) -> Res<PushOutcome> {
    match package.push(None).await? {
        Pushed::Uploaded(outcome) => Ok(outcome),
        Pushed::NothingToPush => panic!("a commit to push"),
    }
}

async fn upstream(package: &Package) -> Res<UpstreamState> {
    Ok(package
        .status(Some(HostConfig::default()))
        .await?
        .upstream_state)
}

async fn current_hash(package: &Package) -> Res<String> {
    Ok(package
        .lineage()
        .await?
        .current_hash()
        .expect("an installed revision")
        .to_string())
}

/// The bytes of `latest` for `namespace` in `bucket_name`.
async fn latest(bucket: &MockRemote, bucket_name: &str, namespace: &Namespace) -> Res<Vec<u8>> {
    let uri = S3Uri::from_str(&format!(
        "s3://{bucket_name}/.quilt/named_packages/{namespace}/latest"
    ))?;
    Ok(bucket
        .get_object_stream(None, &uri)
        .await?
        .body
        .collect()
        .await?
        .to_vec())
}

/// Where a package is after the other client published over the revision
/// this one installed.
struct Moved {
    bucket: Arc<MockRemote>,
    bucket_name: String,
    namespace: Namespace,
    /// The revision both clients started from.
    v1: String,
    /// The other client's revision, now `latest`.
    v2: String,
    /// This client, still at `v1`.
    ours: Package,
    _clients: Vec<Client>,
    _scratch: Option<tempfile::TempDir>,
}

impl Moved {
    /// The other client published while we had a local commit; our push
    /// uploads the revision and leaves `latest` at theirs.
    async fn push_local_commit(&self) -> Res<String> {
        write_file(&self.ours, "data.txt", "mine\n").await?;
        let local = commit(&self.ours, "ours").await?;
        let pushed = push(&self.ours).await?;
        assert!(!pushed.certified_latest);
        assert_eq!(pushed.manifest_uri.hash, local);
        assert_eq!(
            latest(&self.bucket, &self.bucket_name, &self.namespace).await?,
            self.v2.as_bytes(),
            "our push moved latest over the other client's revision"
        );
        assert_eq!(upstream(&self.ours).await?, UpstreamState::Diverged);
        Ok(local)
    }

    async fn assert_promote_wins(&self) -> Res {
        let local = self.push_local_commit().await?;
        let promoted = self.ours.certify_latest().await?;
        assert_eq!(promoted.hash, local);
        assert_eq!(
            latest(&self.bucket, &self.bucket_name, &self.namespace).await?,
            local.as_bytes()
        );
        assert_eq!(upstream(&self.ours).await?, UpstreamState::UpToDate);
        Ok(())
    }

    async fn assert_reset_takes_theirs(&self) -> Res {
        self.push_local_commit().await?;
        let reset = self.ours.reset_to_latest().await?;
        assert_eq!(reset.hash, self.v2);
        assert_eq!(current_hash(&self.ours).await?, self.v2);
        assert_eq!(
            latest(&self.bucket, &self.bucket_name, &self.namespace).await?,
            self.v2.as_bytes()
        );
        assert_eq!(upstream(&self.ours).await?, UpstreamState::UpToDate);
        Ok(())
    }

    /// No local changes, so the package is only behind, and a pull lands
    /// on the other client's revision.
    async fn assert_behind_then_pull(&self) -> Res {
        assert_eq!(current_hash(&self.ours).await?, self.v1);
        assert_eq!(upstream(&self.ours).await?, UpstreamState::Behind);
        self.ours
            .pull(Some(HostConfig::default()), SyncScope::IndividualFiles)
            .await?;
        assert_eq!(current_hash(&self.ours).await?, self.v2);
        assert_eq!(read_file(&self.ours, "data.txt").await?, "two\n");
        assert_eq!(upstream(&self.ours).await?, UpstreamState::UpToDate);
        Ok(())
    }
}

/// The other client is a second domain on the same bucket.
mod two_domains {
    use super::*;
    use test_log::test;

    /// Both clients install v1; the other one then pushes v2.
    async fn moved() -> Res<Moved> {
        let bucket = Arc::new(MockRemote::default());
        let namespace: Namespace = ("f", "a").into();
        let scratch = tempfile::TempDir::new()?;
        let data = publish_row(&bucket, scratch.path(), "data.txt", "d1", b"one\n").await?;
        let same = publish_row(&bucket, scratch.path(), "same.txt", "s1", b"same\n").await?;
        let v1 = publish_manifest(&bucket, vec![data, same]).await?;
        bucket
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/named_packages/f/a/latest"))?,
                v1.as_bytes().to_vec(),
            )
            .await?;

        let us = Client::new(&bucket).await?;
        let them = Client::new(&bucket).await?;
        let ours = us.install(BUCKET, &namespace, &v1).await?;
        let theirs = them.install(BUCKET, &namespace, &v1).await?;

        write_file(&theirs, "data.txt", "two\n").await?;
        let v2 = commit(&theirs, "theirs").await?;
        assert!(push(&theirs).await?.certified_latest);

        Ok(Moved {
            bucket,
            bucket_name: BUCKET.to_string(),
            namespace,
            v1,
            v2,
            ours,
            _clients: vec![us, them],
            _scratch: Some(scratch),
        })
    }

    #[test(tokio::test)]
    async fn push_over_moved_latest_diverges_and_promote_wins() -> Res {
        moved().await?.assert_promote_wins().await
    }

    #[test(tokio::test)]
    async fn push_over_moved_latest_diverges_and_reset_takes_theirs() -> Res {
        moved().await?.assert_reset_takes_theirs().await
    }

    #[test(tokio::test)]
    async fn moved_latest_without_local_changes_is_behind_and_pulls() -> Res {
        moved().await?.assert_behind_then_pull().await
    }
}

/// The other client is quilt3: the bucket holds the objects it wrote.
mod quilt3_bucket {
    use super::*;
    use test_log::test;

    const BUCKET_NAME: &str = "quilt3-bucket";

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quilt3-bucket")
    }

    /// Every file under `dir`, as its path relative to `dir` in `/` form.
    fn keys(dir: &Path) -> Res<Vec<String>> {
        let mut keys = Vec::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(next) = todo.pop() {
            for entry in std::fs::read_dir(next)? {
                let path = entry?.path();
                if path.is_dir() {
                    todo.push(path);
                } else {
                    let relative = path.strip_prefix(dir).expect("under dir");
                    let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
                    keys.push(parts.join("/"));
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// Puts what quilt3's push number `step` wrote into `bucket`, byte for
    /// byte. Data objects go in at the version its manifest names.
    async fn load(bucket: &MockRemote, step: &str) -> Res<String> {
        let dir = fixtures().join(step);
        let keys = keys(&dir)?;
        let mut versioned = std::collections::HashMap::new();
        let mut manifest_hash = None;
        for key in &keys {
            if let Some(hash) = key.strip_prefix(".quilt/packages/") {
                manifest_hash = Some(hash.to_string());
                let manifest = Manifest::from_path(&LocalStorage::new(), &dir.join(key)).await?;
                for row in manifest.rows {
                    let uri = S3Uri::from_str(&row.physical_key)?;
                    versioned.insert(uri.key.clone(), uri);
                }
            }
        }
        for key in &keys {
            let uri = if key.starts_with(".quilt/") {
                S3Uri::from_str(&format!("s3://{BUCKET_NAME}/{key}"))?
            } else {
                versioned
                    .get(key)
                    .unwrap_or_else(|| panic!("{key} is in no manifest of {step}"))
                    .clone()
            };
            let body = tokio::fs::read(dir.join(key)).await?;
            bucket.put_object(None, &uri, body).await?;
        }
        Ok(manifest_hash.expect("each step holds one manifest"))
    }

    /// quilt3 pushed v1, we installed it, then quilt3 pushed v2.
    async fn moved() -> Res<Moved> {
        let bucket = Arc::new(MockRemote::default());
        let namespace: Namespace = ("quilt3", "pkg").into();
        let v1 = load(&bucket, "v1").await?;
        assert_eq!(
            latest(&bucket, BUCKET_NAME, &namespace).await?,
            v1.as_bytes()
        );

        let us = Client::new(&bucket).await?;
        let ours = us.install(BUCKET_NAME, &namespace, &v1).await?;
        assert_eq!(upstream(&ours).await?, UpstreamState::UpToDate);

        let v2 = load(&bucket, "v2").await?;
        Ok(Moved {
            bucket,
            bucket_name: BUCKET_NAME.to_string(),
            namespace,
            v1,
            v2,
            ours,
            _clients: vec![us],
            _scratch: None,
        })
    }

    #[test(tokio::test)]
    async fn push_over_quilt3_latest_diverges_and_promote_wins() -> Res {
        moved().await?.assert_promote_wins().await
    }

    #[test(tokio::test)]
    async fn push_over_quilt3_latest_diverges_and_reset_takes_theirs() -> Res {
        moved().await?.assert_reset_takes_theirs().await
    }

    #[test(tokio::test)]
    async fn quilt3_latest_without_local_changes_is_behind_and_pulls() -> Res {
        moved().await?.assert_behind_then_pull().await
    }

    /// Our push writes its pointers in the form quilt3 wrote its own: named
    /// in whole seconds, holding the bare hash with no newline.
    #[test(tokio::test)]
    async fn our_pointers_take_quilt3s_form() -> Res {
        let moved = moved().await?;
        let local = moved.push_local_commit().await?;
        let dir = format!("{BUCKET_NAME}/.quilt/named_packages/{}", moved.namespace);
        let mut entries = moved.bucket.storage.read_dir(&dir).await?;
        let mut pointers = std::collections::BTreeMap::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            pointers.insert(name, tokio::fs::read(entry.path()).await?);
        }
        let hashes: std::collections::BTreeSet<_> = pointers
            .iter()
            .filter(|(name, _)| name.as_str() != "latest")
            .map(|(name, body)| {
                assert!(name.bytes().all(|b| b.is_ascii_digit()), "pointer {name}");
                String::from_utf8(body.clone()).expect("a pointer is text")
            })
            .collect();
        let expected: std::collections::BTreeSet<_> = [moved.v1.clone(), moved.v2.clone(), local]
            .into_iter()
            .collect();
        assert_eq!(hashes, expected);
        Ok(())
    }
}

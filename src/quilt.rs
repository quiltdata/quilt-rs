use std::{
    collections::{hash_map::RandomState, BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
};

use aws_sdk_s3::{
    error::SdkError,
    types::{ChecksumAlgorithm, CompletedMultipartUpload, CompletedPart},
};
use aws_smithy_types::byte_stream::{ByteStream, Length};
use base64::{prelude::BASE64_STANDARD, Engine};
use multihash::Multihash;
use parquet::data_type::AsBytes;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    fs::{create_dir_all, read_dir, remove_dir_all, File},
    io::{AsyncReadExt, AsyncWriteExt},
};
use url::Url;

pub mod lineage;
pub mod manifest;
pub mod storage;
pub mod uri;

use crate::{
    quilt4::{
        checksum::{
            self, calculate_sha256_checksum, calculate_sha256_chunked_checksum, get_checksum_chunksize_and_parts
        },
        table::HEADER_ROW,
    },
    s3_utils, Row4, Table, UPath,
};

use self::manifest::{MULTIHASH_SHA256, MULTIHASH_SHA256_CHUNKED};
pub use self::{
    // context::Context,
    lineage::{CommitState, DomainLineage, PackageLineage, PathState},
    manifest::{ContentHash, Manifest, ManifestHeader, ManifestRow},
    storage::{fs, s3},
    uri::{RevisionPointer, S3PackageURI},
};

const MANIFEST_DIR: &str = ".quilt/packages";
const TAGS_DIR: &str = ".quilt/named_packages";
const OBJECTS_DIR: &str = ".quilt/objects";
const LINEAGE_FILE: &str = ".quilt/data.json";
const INSTALLED_DIR: &str = ".quilt/installed";

const MULTIPART_THRESHOLD: u64 = checksum::MULTIPART_THRESHOLD;

pub fn tag_key(namespace: &str, tag: &str) -> String {
    format!("{TAGS_DIR}/{namespace}/{tag}")
}

pub fn tag_uri(bucket: &str, namespace: &str, tag: &str) -> s3::S3Uri {
    s3::S3Uri {
        bucket: bucket.to_owned(),
        key: tag_key(namespace, tag),
        version: None,
    }
}

fn parquet_manifest_filename(top_hash: &str) -> String {
    format!("1220{}.parquet", top_hash)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteManifest {
    pub bucket: String,
    pub namespace: String,
    pub hash: String,
}

impl RemoteManifest {
    pub async fn resolve(uri: &S3PackageURI) -> Result<Self, String> {
        // resolve the actual hash
        let top_hash = match &uri.revision {
            RevisionPointer::Hash(top_hash) => top_hash.clone(),
            RevisionPointer::Tag(tag) => {
                tag_uri(&uri.bucket, &uri.namespace, tag)
                    .get_contents()
                    .await?
            }
        };

        Ok(Self {
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
            hash: top_hash,
        })
    }

    pub async fn resolve_latest(&self) -> Result<String, String> {
        tag_uri(&self.bucket, &self.namespace, "latest")
            .get_contents()
            .await
    }

    pub async fn update_latest(&self, hash: String) -> Result<(), String> {
        tag_uri(&self.bucket, &self.namespace, "latest")
            .put_contents(hash.into_bytes())
            .await
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CachedManifest {
    pub domain: LocalDomain,
    pub bucket: String,
    pub hash: String,
}

impl CachedManifest {
    pub async fn read(&self) -> Result<Table, String> {
        let pathbuf = self.domain.manifest_cache_path(&self.bucket, &self.hash);
        let path = UPath::Local(pathbuf);
        let table = Table::read_from_upath(&path)
            .await
            .map_err(|err| err.to_string())?;
        Ok(table)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledManifest {
    pub package: InstalledPackage,
    pub hash: String,
}

impl InstalledManifest {
    pub async fn read(&self) -> Result<Table, String> {
        let pathbuf = self
            .package
            .domain
            .installed_manifest_path(&self.package.namespace, &self.hash);
        let path = UPath::Local(pathbuf);
        let table = Table::read_from_upath(&path)
            .await
            .map_err(|err| err.to_string())?;
        Ok(table)
    }
}

// XXX: is this necessary?
#[derive(Debug, PartialEq, Eq)]
struct S3Domain {
    bucket: String,
}

impl From<&S3PackageURI> for S3Domain {
    fn from(uri: &S3PackageURI) -> Self {
        Self {
            bucket: uri.bucket.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDomain {
    root_dir: PathBuf,
}

impl LocalDomain {
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    pub fn make_cached_manifest(
        &self,
        bucket: impl AsRef<str>,
        hash: impl AsRef<str>,
    ) -> CachedManifest {
        CachedManifest {
            domain: self.clone(),
            bucket: String::from(bucket.as_ref()),
            hash: String::from(hash.as_ref()),
        }
    }

    pub fn manifest_cache_path(&self, bucket: &str, hash: &str) -> PathBuf {
        self.root_dir.join(MANIFEST_DIR).join(bucket).join(hash)
    }

    pub fn working_folder(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(namespace)
    }

    // TODO: use tokio::fs::read
    pub async fn read_lineage(&self) -> Result<DomainLineage, String> {
        let lineage_path = self.root_dir.join(LINEAGE_FILE);
        let contents = fs::read_to_string(&lineage_path).await.or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Ok("{}".into())
            } else {
                Err(format!(
                    "Failed to read the lineage file: {}",
                    err.to_string()
                ))
            }
        })?;

        DomainLineage::try_from(&contents[..])
    }

    pub async fn write_lineage(&self, lineage: &DomainLineage) -> Result<(), String> {
        let lineage_path = self.root_dir.join(LINEAGE_FILE);
        let contents = serde_json::to_string_pretty(lineage).map_err(|err| err.to_string())?;
        fs::write(lineage_path, contents.as_bytes())
            .await
            .map_err(|err| err.to_string())
    }

    pub async fn cache_remote_manifest(
        &self,
        manifest: &RemoteManifest,
    ) -> Result<CachedManifest, String> {
        // check if the manifest is already cached
        // if not, download and cache it
        // return cached manifest

        let cache_path = self.manifest_cache_path(&manifest.bucket, &manifest.hash);

        // TODO: who is responsible for this?
        create_dir_all(&cache_path.parent().unwrap())
            .await
            .map_err(|err| err.to_string())?;

        if !fs::exists(&cache_path).await {
            // Does not exist yet
            let client = crate::s3_utils::get_client_for_bucket(&manifest.bucket).await?;

            let result = client
                .get_object()
                .bucket(&manifest.bucket)
                .key(format!(
                    "{}/{}",
                    MANIFEST_DIR,
                    parquet_manifest_filename(&manifest.hash)
                ))
                .send()
                .await;

            match result {
                Ok(output) => {
                    let mut contents = Vec::new();
                    output
                        .body
                        .into_async_read()
                        .read_to_end(&mut contents)
                        .await
                        .map_err(|err| err.to_string())?;

                    fs::write(&cache_path, &contents).await.map_err(|err| {
                        format!("Failed to write manifest to {cache_path:?}: {err}")
                    })?;
                }
                Err(SdkError::ServiceError(err)) if err.err().is_no_such_key() => {
                    // Fallback: Download the JSONL manifest.
                    let result = client
                        .get_object()
                        .bucket(&manifest.bucket)
                        .key(format!("{}/{}", MANIFEST_DIR, &manifest.hash))
                        .send()
                        .await
                        .map_err(|err| {
                            err.into_service_error()
                                .meta()
                                .message()
                                .unwrap_or("failed to download s3 object")
                                .to_string()
                        })?;

                    let quilt3_manifest =
                        Manifest::from_file(result.body.into_async_read()).await?;
                    let header = Row4 {
                        name: HEADER_ROW.into(),
                        place: HEADER_ROW.into(),
                        path: None,
                        size: 0,
                        hash: Multihash::default(),
                        info: serde_json::json!({
                            "message": quilt3_manifest.header.message,
                            "version": quilt3_manifest.header.version,
                        }),
                        meta: match quilt3_manifest.header.user_meta {
                            Some(meta) => meta.into(),
                            None => serde_json::Value::Null,
                        },
                    };
                    let mut records = BTreeMap::new();
                    for row in quilt3_manifest.rows {
                        let mut info = row.meta.unwrap_or_default();
                        let meta = info.remove("user_meta").unwrap_or_default();
                        records.insert(
                            row.logical_key.clone(),
                            Row4 {
                                name: row.logical_key,
                                place: row.physical_key,
                                path: None,
                                size: row.size,
                                hash: row.hash.try_into()?,
                                info: info.into(),
                                meta,
                            },
                        );
                    }
                    let table = Table { header, records };
                    table
                        .write_to_upath(&UPath::Local(cache_path))
                        .await
                        .map_err(|err| err.to_string())?;
                }
                Err(err) => {
                    return Err(err
                        .into_service_error()
                        .meta()
                        .message()
                        .unwrap_or("failed to download s3 object")
                        .to_string());
                }
            }
        }

        Ok(CachedManifest {
            domain: self.to_owned(),
            bucket: manifest.bucket.clone(),
            hash: manifest.hash.clone(),
        })
    }

    pub async fn browse_remote_manifest(&self, remote: &RemoteManifest) -> Result<Table, String> {
        self.cache_remote_manifest(remote).await?.read().await
    }

    pub async fn browse_uri(&self, uri: &S3PackageURI) -> Result<Table, String> {
        // resolve uri to the manifest location and hash
        let remote_manifest = RemoteManifest::resolve(uri).await?;
        self.browse_remote_manifest(&remote_manifest).await
    }

    pub fn installed_manifests_path(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(INSTALLED_DIR).join(namespace)
    }

    pub fn installed_manifest_path(&self, namespace: &str, hash: &str) -> PathBuf {
        self.installed_manifests_path(namespace).join(hash)
    }

    async fn create_objects_dir(&self) -> Result<PathBuf, String> {
        let objects_dir = self.root_dir.join(OBJECTS_DIR);
        create_dir_all(&objects_dir)
            .await
            .map_err(|err| err.to_string())?;
        Ok(objects_dir)
    }

    async fn create_working_dir(&self, namespace: &str) -> Result<PathBuf, String> {
        let working_dir = self.working_folder(namespace);
        create_dir_all(&working_dir)
            .await
            .map_err(|err| err.to_string())?;
        Ok(working_dir)
    }

    async fn write_manifest_to_installed_dir(
        &self,
        manifest: &Table,
        namespace: &str,
    ) -> Result<String, String> {
        let hash = manifest.top_hash();
        let installed_manifest_path = self.installed_manifest_path(namespace, &hash);
        create_dir_all(&installed_manifest_path.parent().unwrap())
            .await
            .map_err(|err| err.to_string())?;
        manifest
            .write_to_upath(&UPath::Local(installed_manifest_path))
            .await
            .map_err(|err| err.to_string())?;
        Ok(hash)
    }

    async fn copy_cached_manifest_to_installed_dir(
        &self,
        remote_manifest: &RemoteManifest,
    ) -> Result<String, String> {
        let installed_manifest_path =
            self.installed_manifest_path(&remote_manifest.namespace, &remote_manifest.hash);
        create_dir_all(&installed_manifest_path.parent().unwrap())
            .await
            .map_err(|err| err.to_string())?;
        tokio::fs::copy(
            self.manifest_cache_path(&remote_manifest.bucket, &remote_manifest.hash),
            installed_manifest_path,
        )
        .await
        .map_err(|err| err.to_string())?;
        Ok(remote_manifest.hash.clone())
    }

    async fn write_package_to_lineage(
        &self,
        remote_manifest: &RemoteManifest,
        hash: String,
    ) -> Result<(), String> {
        let lineage: DomainLineage = self.read_lineage().await?;
        let mut lineage = lineage;
        lineage.packages.insert(
            remote_manifest.namespace.to_string(),
            PackageLineage::from_remote(remote_manifest.clone(), hash),
        );
        self.write_lineage(&lineage).await?;
        Ok(())
    }

    async fn install_all_paths(
        &self,
        installed_package: &InstalledPackage,
        manifest: &Table,
    ) -> Result<Vec<PathBuf>, String> {
        let working_dir = installed_package.working_folder();
        let mut keys = Vec::new();
        let mut paths = Vec::new();
        for row in manifest.records.values() {
            keys.push(row.name.clone());
            paths.push(working_dir.join(&row.name))
        }

        installed_package.install_paths(&keys).await?;
        Ok(paths)
    }

    pub async fn install_package(
        &self,
        remote: &RemoteManifest,
    ) -> Result<InstalledPackage, String> {
        // bail if already installed
        // TODO: if compatible (same remote), just return the installed package
        let lineage = self.read_lineage().await?;
        if lineage.packages.contains_key(&remote.namespace) {
            return Err(format!(
                "Package '{}' is already installed",
                remote.namespace
            ));
        }

        self.cache_remote_manifest(remote).await?;
        self.copy_cached_manifest_to_installed_dir(remote).await?;
        self.create_objects_dir().await?;
        self.create_working_dir(&remote.namespace).await?;

        // Resolve and record latest manifest hash
        let latest_hash = remote.resolve_latest().await?;
        self.write_package_to_lineage(remote, latest_hash).await?;

        Ok(InstalledPackage {
            domain: self.to_owned(),
            namespace: remote.namespace.clone(),
        })
    }

    pub async fn uninstall_package(&self, namespace: impl AsRef<str>) -> Result<(), String> {
        let namespace = namespace.as_ref();
        let mut lineage = self.read_lineage().await?;

        lineage
            .packages
            .remove(namespace)
            .ok_or("Package not installed".to_string())?;

        self.write_lineage(&lineage).await?;

        if let Err(err) = remove_dir_all(self.installed_manifests_path(namespace)).await {
            println!("Failed to remove installed manifests: {err}");
        }
        if let Err(err) = remove_dir_all(self.working_folder(namespace)).await {
            println!("Failed to remove working directory: {err}");
        }

        // TODO: Remove object files? But need to make sure no other manifest uses them.

        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Result<Vec<InstalledPackage>, String> {
        let lineage = self.read_lineage().await?;
        let mut namespaces: Vec<String> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let packages = namespaces
            .into_iter()
            .map(|namespace| InstalledPackage {
                domain: self.to_owned(),
                namespace,
            })
            .collect();
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &str,
    ) -> Result<Option<InstalledPackage>, String> {
        let lineage = self.read_lineage().await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(InstalledPackage {
                domain: self.to_owned(),
                namespace: namespace.to_owned(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn package_s3_prefix(
        &self,
        namespace: &str,
        uri: &s3::S3Uri,
    ) -> Result<(InstalledPackage, Option<Vec<PathBuf>>), String> {
        let manifest = package_s3_prefix(uri).await?;

        let hash = self
            .write_manifest_to_installed_dir(&manifest, namespace)
            .await?;

        self.create_objects_dir().await?;
        self.create_working_dir(namespace).await?;

        self.write_package_to_lineage(
            &RemoteManifest {
                // HACK: there is no remote manifest
                bucket: uri.bucket.clone(),
                namespace: namespace.to_string(),
                hash: hash.clone(),
            },
            hash.clone(),
        )
        .await?;

        let installed_package = InstalledPackage {
            domain: self.to_owned(),
            namespace: namespace.to_string(),
        };
        let paths = self
            .install_all_paths(&installed_package, &manifest)
            .await?;

        Ok((
            installed_package,
            if paths.is_empty() { None } else { Some(paths) },
        ))
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Change<T> {
    pub current: Option<T>,
    pub previous: Option<T>,
}

pub type ChangeSet<K, T> = BTreeMap<K, Change<T>>;

#[derive(Debug, PartialEq, Eq, Default, Serialize)]
pub struct UpstreamState {
    commit_pending: bool, // whether there's a commit to be pushed
    behind: bool,         // whether **base** and **latest** revisions differ
    ahead: bool,          // whether **base** and **current** revisions differ
}

impl UpstreamState {
    pub fn from_lineage(lineage: &PackageLineage) -> Self {
        Self {
            commit_pending: lineage.commit.is_some(),
            behind: lineage.base_hash != lineage.latest_hash,
            ahead: lineage.base_hash != lineage.current_hash(),
        }
    }
}

// XXX: do we  actually need this? two-flag (ahead-behind) logic seems simple enough
#[derive(Debug, PartialEq, Eq, Default, Serialize)]
pub enum UpstreamDiscreteState {
    #[default]
    UpToDate,
    Behind,
    Ahead,
    Diverged,
}

impl From<&UpstreamState> for UpstreamDiscreteState {
    fn from(upstream: &UpstreamState) -> Self {
        match (upstream.ahead, upstream.behind) {
            (false, false) => Self::UpToDate,
            (false, true) => Self::Behind,
            (true, false) => Self::Ahead,
            (true, true) => Self::Diverged,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackageFileFingerprint {
    pub size: u64,
    pub hash: Multihash<256>,
}

#[derive(Debug, PartialEq, Default)]
pub struct InstalledPackageStatus {
    // current commit vs upstream state
    pub upstream: UpstreamState,
    pub upstream_state: UpstreamDiscreteState,
    pub dirty: bool, // whether there are uncommitted changes
    // file changes vs current commit
    pub changes: ChangeSet<String, PackageFileFingerprint>,
    // XXX: meta?
}

impl InstalledPackageStatus {
    pub fn new(
        upstream: UpstreamState,
        changes: ChangeSet<String, PackageFileFingerprint>,
    ) -> Self {
        Self {
            upstream_state: UpstreamDiscreteState::from(&upstream),
            upstream,
            dirty: !changes.is_empty(),
            changes,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage {
    pub domain: LocalDomain,
    pub namespace: String,
}

impl InstalledPackage {
    pub async fn lineage(&self) -> Result<PackageLineage, String> {
        self.domain
            .read_lineage()
            .await?
            .packages
            .get(&self.namespace)
            .ok_or("not found".to_string())
            // TODO: just move the value without cloning
            .cloned()
    }

    pub async fn write_lineage(&self, lineage: PackageLineage) -> Result<(), String> {
        let mut domain_lineage = self.domain.read_lineage().await?;
        domain_lineage
            .packages
            .insert(self.namespace.clone(), lineage);
        self.domain.write_lineage(&domain_lineage).await
    }

    pub async fn paths(&self) -> Result<Vec<String>, String> {
        self.lineage().await.map(|l| l.paths.into_keys().collect())
    }

    pub fn make_installed_manifest(&self, hash: impl AsRef<str>) -> InstalledManifest {
        InstalledManifest {
            package: self.to_owned(),
            hash: String::from(hash.as_ref()),
        }
    }

    pub async fn manifest(&self) -> Result<InstalledManifest, String> {
        // read recorded hash
        // get installed manifest
        self.lineage()
            .await
            .map(|l| self.make_installed_manifest(l.current_hash()))
    }

    pub fn working_folder(&self) -> PathBuf {
        self.domain.working_folder(&self.namespace)
    }

    pub async fn uninstall(&self) -> Result<(), String> {
        self.domain.uninstall_package(&self.namespace).await
    }

    pub async fn status(&self) -> Result<InstalledPackageStatus, String> {
        // compute the status based on the following sources:
        //   - the cached manifest
        //   - paths
        //   - working directory state
        // installed entries marked as "installed" (initially as "downloading")
        // modified entries marked as "modified", etc

        let mut lineage = self.lineage().await?;

        // try updating the latest hash
        if let Ok(latest_hash) = lineage.remote.resolve_latest().await {
            lineage.latest_hash = latest_hash;
            self.write_lineage(lineage.clone()).await?;
        }

        let table = self.manifest().await?.read().await?;

        let work_dir = self.working_folder();

        let mut orig_paths = HashMap::new();
        for path in lineage.paths.keys() {
            let row = table.get_row(path).ok_or("no such path")?;
            orig_paths.insert(PathBuf::from(path), (row.hash.clone(), row.size));
        }

        let mut queue = VecDeque::new();
        queue.push_back(work_dir.clone());

        let mut changes = ChangeSet::new();

        while let Some(dir) = queue.pop_front() {
            let mut dir_entries = match read_dir(&dir).await {
                Ok(dir_entries) => dir_entries,
                Err(err) => {
                    println!("Failed to read directory {:?}: {}", dir, err);
                    continue;
                }
            };

            while let Some(dir_entry) = dir_entries
                .next_entry()
                .await
                .map_err(|err| err.to_string())?
            {
                let file_path = dir_entry.path();
                let file_type = dir_entry.file_type().await.map_err(|err| err.to_string())?;

                if file_type.is_dir() {
                    queue.push_back(file_path);
                } else if file_type.is_file() {
                    let file = File::open(&file_path)
                        .await
                        .map_err(|err| err.to_string())?;
                    let file_metadata = file.metadata().await.map_err(|err| err.to_string())?;

                    let relative_path = file_path.strip_prefix(&work_dir).unwrap();
                    if let Some((orig_hash, orig_size)) = orig_paths.remove(relative_path) {
                        let file_hash = match orig_hash.code() {
                            MULTIHASH_SHA256_CHUNKED => {
                                let hash =
                                    calculate_sha256_chunked_checksum(file, file_metadata.len())
                                        .await
                                        .map_err(|err| err.to_string())?;
                                Multihash::wrap(MULTIHASH_SHA256_CHUNKED, hash.as_ref()).unwrap()
                            }
                            _ => {
                                let hash = calculate_sha256_checksum(file)
                                    .await
                                    .map_err(|err| err.to_string())?;
                                Multihash::wrap(MULTIHASH_SHA256, hash.as_ref()).unwrap()
                            }
                        };

                        if file_hash != orig_hash {
                            changes.insert(
                                relative_path.display().to_string(),
                                Change {
                                    current: Some(PackageFileFingerprint {
                                        size: file_metadata.len(),
                                        hash: file_hash,
                                    }),
                                    previous: Some(PackageFileFingerprint {
                                        size: orig_size,
                                        hash: orig_hash,
                                    }),
                                },
                            );
                        }
                    } else {
                        let sha256_hash = calculate_sha256_checksum(file)
                            .await
                            .map_err(|err| err.to_string())?;
                        let file_hash =
                            Multihash::wrap(MULTIHASH_SHA256, sha256_hash.as_ref()).unwrap();
                        changes.insert(
                            relative_path.display().to_string(),
                            Change {
                                current: Some(PackageFileFingerprint {
                                    size: file_metadata.len(),
                                    hash: file_hash,
                                }),
                                previous: None,
                            },
                        );
                    }
                } else {
                    println!("Unexpected file type: {}", file_path.display());
                }
            }
        }

        for (orig_path, (orig_hash, orig_size)) in orig_paths {
            changes.insert(
                orig_path.display().to_string(),
                Change {
                    current: None,
                    previous: Some(PackageFileFingerprint {
                        size: orig_size,
                        hash: orig_hash,
                    }),
                },
            );
        }

        Ok(InstalledPackageStatus::new(
            UpstreamState::from_lineage(&lineage),
            changes,
        ))
    }

    pub async fn install_paths(&self, paths: &Vec<String>) -> Result<(), String> {
        if paths.len() == 0 {
            return Ok(());
        }

        let mut lineage = self.lineage().await?;

        // TODO: what happens if paths are already installed? Ignore, or error?
        if !HashSet::<String, RandomState>::from_iter(lineage.paths.keys().cloned())
            .is_disjoint(&HashSet::from_iter(paths.to_owned()))
        {
            return Err(format!("duplicate paths"));
        }

        let objects_dir = self.domain.root_dir.join(OBJECTS_DIR);
        let working_dir = self.working_folder();

        // for each path in paths:
        //   get entry from installed manifest
        //   cache the entry into identity cache (if not there)
        //   replace entry's physical key in the manifest with the cached physical key
        //
        // write the adjusted manifest into the installed manifest path
        // copy the selected paths into the working folder
        //
        // record installation into the lineage:
        //   add installed package entry:
        //     remote: RemoteManifest

        let mut table = self.manifest().await?.read().await?;

        for path in paths {
            // TODO: Consider using a hashmap or treemap for manifest.rows
            let row = table.records.get_mut(path).ok_or("no such path")?;

            let parsed_url = Url::parse(&row.place).map_err(|err| err.to_string())?;
            if parsed_url.scheme() != "s3" {
                return Err("invalid scheme".into());
            }
            let bucket = parsed_url.host_str().ok_or("missing bucket")?;
            let key = percent_encoding::percent_decode_str(&parsed_url.path()[1..])
                .decode_utf8()
                .map_err(|err| err.to_string())?;
            let query: HashMap<_, _> = parsed_url.query_pairs().into_owned().collect();
            let version_id = query.get("versionId").ok_or("missing versionId")?; // TODO

            let object_dest = objects_dir.join(hex::encode(row.hash.digest()));

            if !fs::exists(&object_dest).await {
                let mut file = File::create(&object_dest)
                    .await
                    .map_err(|err| err.to_string())?;

                let client = s3_utils::get_client_for_bucket(bucket.into()).await?;

                let mut object = client
                    .get_object()
                    .bucket(bucket)
                    .key(key)
                    .version_id(version_id)
                    .send()
                    .await
                    .map_err(|err| {
                        err.into_service_error()
                            .meta()
                            .message()
                            .unwrap_or("failed to download")
                            .to_string()
                    })?;

                while let Some(bytes) = object
                    .body
                    .try_next()
                    .await
                    .map_err(|err| err.to_string())?
                {
                    file.write_all(&bytes)
                        .await
                        .map_err(|err| err.to_string())?;
                }
                file.flush().await.map_err(|err| err.to_string())?;
            }

            row.place = Url::from_file_path(&object_dest).unwrap().to_string();

            let working_dest = working_dir.join(&row.name);
            let parent_dir = working_dest.parent();
            if let Some(_) = parent_dir {
                tokio::fs::create_dir_all(parent_dir.unwrap())
                    .await
                    .map_err(|err| err.to_string())?;
            }
            tokio::fs::copy(&object_dest, &working_dest)
                .await
                .map_err(|err| err.to_string())?;
            let timestamp = fs::get_file_modified_ts(&working_dest).await?;
            lineage.paths.insert(
                row.name.to_owned(),
                PathState {
                    timestamp,
                    hash: row.hash.to_owned(),
                },
            );
        }

        // save the manifest
        // TODO: Write to a temporary file first.
        let installed_manifest_path = self
            .domain
            .installed_manifest_path(&self.namespace, lineage.current_hash());

        table
            .write_to_upath(&UPath::Local(installed_manifest_path))
            .await
            .map_err(|err| err.to_string())?;

        self.write_lineage(lineage).await?;

        Ok(())
    }

    pub async fn uninstall_paths(&self, paths: &Vec<String>) -> Result<(), String> {
        println!("uninstall_paths: {paths:?}");

        let mut lineage = self.lineage().await?;

        let working_dir = self.working_folder();
        for path in paths {
            lineage.paths.remove(path).ok_or("path is not installed")?;

            let working_path = working_dir.join(path);
            match tokio::fs::remove_file(working_path).await {
                Ok(()) => (),
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        return Err(err.to_string());
                    }
                }
            };
        }

        self.write_lineage(lineage).await?;

        // TODO: Remove unused files in OBJECTS_DIR?

        Ok(())
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Result<(), String> {
        println!("revert_paths: {paths:?}");
        Err("not implemented".into())
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<manifest::JsonObject>,
    ) -> Result<(), String> {
        println!("commit: {message:?}, {user_meta:?}");
        // create a new manifest based on the stored version

        // for each modified file:
        //   - compute the new hash
        //   - store in the identity cache at $LOCAL/.quilt/objects/<hash>
        //   - update the modified entries in the manifest with the new physical keys
        //     pointing to the new objects in the identity cache
        //   - ? set entry.meta.pulled_hashes to prevous object hash?
        //   - ? set entry.meta.remote_key to the remote's physical key?

        // compute the new top hash
        // store the new manifest under the new top hash at $LOCAL/.quilt/packages/<hash>
        // XXX: prefix with the namespace?
        // XXX: what to do on collisions?
        //      e.g. when a file was changed, committed, and then reverted

        // store revision pointers to the newly created manifest
        //   - in the local registry??
        //   - in the lineage
        //     - commit:
        //       - timestamp
        //       - user ?
        //       - multihash: new_top_hash
        //       - pulled_hashes: [old_top_hash] ?
        //       - paths:
        //         - [modified file's path]:
        //           - multihash
        //           # XXX: do we actually need this? can be inferred from namepsace + logical key
        //           - remote_key: "s3://..." # no version id
        //           - local_key: $LOCAL/.quilt/objects/<hash>
        //           - pulled_hashes: [old_hash] ?
        // NOTE: each commit MUST include all paths from prior commits
        //       (since the last pull, until reset by a sync)

        let mut lineage = self.domain.read_lineage().await?;
        let package_lineage = lineage
            .packages
            .get_mut(&self.namespace)
            .ok_or("not found")?;

        // TODO: Maybe have the user pass this as an argument?
        let status = self.status().await?;

        let objects_dir = self.domain.root_dir.join(OBJECTS_DIR);
        // TODO: This should really be done when the domain is created.
        create_dir_all(&objects_dir)
            .await
            .map_err(|err| err.to_string())?;

        let work_dir = self.working_folder();

        let mut table = self.manifest().await?.read().await?;

        for (logical_key, Change { current, previous }) in status.changes {
            if let Some(previous) = previous {
                let removed = table
                    .records
                    .remove(&logical_key)
                    .ok_or(format!("cannot remove {}", logical_key))?;
                if removed.size != previous.size || removed.hash != previous.hash {
                    return Err(format!(
                        "unexpected size or hash for removed {}",
                        logical_key
                    ));
                }
                package_lineage.paths.remove(&logical_key);
            }
            if let Some(current) = current {
                let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
                let new_physical_key = Url::from_file_path(&object_dest).unwrap().into();

                if table
                    .records
                    .insert(
                        logical_key.to_owned(),
                        Row4 {
                            name: logical_key.to_owned(),
                            place: new_physical_key,
                            path: None,
                            size: current.size,
                            hash: current.hash.clone(),
                            info: serde_json::Value::default(),
                            meta: serde_json::Value::default(),
                        },
                    )
                    .is_some()
                {
                    return Err(format!("cannot overwrite {}", logical_key));
                }

                let work_dest = work_dir.join(&logical_key);
                if !fs::exists(&object_dest).await {
                    tokio::fs::copy(&work_dest, object_dest)
                        .await
                        .map_err(|err| err.to_string())?;
                }
                package_lineage.paths.insert(
                    logical_key,
                    PathState {
                        timestamp: fs::get_file_modified_ts(&work_dest).await?,
                        hash: current.hash,
                    },
                );
            }
        }

        table.header.info = json!({
            "message": message,
            "version": "v0",
        });
        if let Some(user_meta) = user_meta {
            table.header.meta = user_meta.into();
        }

        let new_top_hash = table.top_hash();

        let new_manifest_path = self
            .domain
            .installed_manifest_path(&self.namespace, &new_top_hash);

        table
            .write_to_upath(&UPath::Local(new_manifest_path))
            .await
            .map_err(|err| err.to_string())?;

        let mut prev_hashes = Vec::new();
        if let Some(commit) = &package_lineage.commit {
            prev_hashes.push(commit.hash.to_owned());
            prev_hashes.extend(commit.prev_hashes.to_owned());
        }
        let commit = CommitState {
            hash: new_top_hash,
            timestamp: chrono::Utc::now(),
            prev_hashes,
        };
        package_lineage.commit = Some(commit);

        self.domain.write_lineage(&lineage).await?;

        Ok(())
    }

    pub async fn push(&self) -> Result<(), String> {
        let mut lineage = self.lineage().await?;

        let commit = match lineage.commit {
            None => return Ok(()), // nothing to commit
            Some(commit) => commit,
        };

        let remote = &lineage.remote;

        let mut local_manifest = self.manifest().await?.read().await?;
        let remote_manifest = self.domain.browse_remote_manifest(remote).await?;

        // ## copy data
        // Copy each of the _modified_ paths from their local_key to remote_key,
        // keeping track of the resulting versionIds
        //
        // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)
        let client = crate::s3_utils::get_client_for_bucket(&remote.bucket).await?;

        // ignore removed items, upload changed and new items
        for row in local_manifest.records.values_mut() {
            if let Some(remote_row) = remote_manifest.records.get(&row.name) {
                if remote_row.eq(row) {
                    row.place = remote_row.place.to_owned();
                    continue;
                }
            }

            let local_url = Url::parse(&row.place).unwrap();
            let file_path: PathBuf = local_url.to_file_path().unwrap();

            let s3_key = format!("{}/{}", self.namespace, row.name);
            println!("uploading to s3({}): {}", remote.bucket, s3_key);

            // TODO: upload in parallel. use a stream?
            let (version_id, checksum) = if row.size < MULTIPART_THRESHOLD {
                let body = ByteStream::read_from()
                    .path(&file_path)
                    .build()
                    .await
                    .map_err(|err| err.to_string())?;

                let response = client
                    .put_object()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .body(body)
                    .checksum_algorithm(ChecksumAlgorithm::Sha256)
                    .send()
                    .await
                    .map_err(|err| err.to_string())?;

                let s3_checksum_b64 = response.checksum_sha256.ok_or("missing checksum")?;

                let s3_checksum = BASE64_STANDARD
                    .decode(s3_checksum_b64)
                    .map_err(|err| err.to_string())?;

                let checksum = if row.size == 0 {
                    // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
                    // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
                    s3_checksum
                } else {
                    calculate_sha256_checksum(s3_checksum.as_ref())
                        .await
                        .unwrap()
                        .to_vec()
                };

                (response.version_id, checksum)
            } else {
                let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(row.size);
                let upload_id = client
                    .create_multipart_upload()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .checksum_algorithm(ChecksumAlgorithm::Sha256)
                    .send()
                    .await
                    .map_err(|err| err.to_string())?
                    .upload_id
                    .ok_or("failed to get an UploadId")?;

                let mut parts: Vec<CompletedPart> = Vec::new();
                for chunk_idx in 0..num_chunks {
                    let part_number = chunk_idx as i32 + 1;
                    let offset = chunk_idx * chunksize;
                    let length = chunksize.min(row.size - offset);
                    let chunk_body = ByteStream::read_from()
                        .path(&file_path)
                        .offset(offset)
                        .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
                        .build()
                        .await
                        .map_err(|err| err.to_string())?;
                    let part_response = client
                        .upload_part()
                        .bucket(&remote.bucket)
                        .key(&s3_key)
                        .upload_id(&upload_id)
                        .part_number(part_number)
                        .checksum_algorithm(ChecksumAlgorithm::Sha256)
                        .body(chunk_body)
                        .send()
                        .await
                        .map_err(|err| err.to_string())?;
                    parts.push(
                        CompletedPart::builder()
                            .part_number(part_number)
                            .e_tag(part_response.e_tag.unwrap_or_default())
                            .checksum_sha256(part_response.checksum_sha256.unwrap_or_default())
                            .build(),
                    );
                }

                let response = client
                    .complete_multipart_upload()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .upload_id(&upload_id)
                    .multipart_upload(
                        CompletedMultipartUpload::builder()
                            .set_parts(Some(parts))
                            .build(),
                    )
                    .send()
                    .await
                    .map_err(|err| err.to_string())?;

                let s3_checksum = response.checksum_sha256.ok_or("missing checksum")?;
                let (checksum_b64, _) = s3_checksum.split_once("-").ok_or("unexpected checksum")?;
                let checksum = BASE64_STANDARD
                    .decode(checksum_b64)
                    .map_err(|err| err.to_string())?;

                (response.version_id, checksum)
            };

            // Update the manifest with the sha2-256-chunked checksum.
            row.hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_ref()).unwrap();

            let remote_url = make_s3_url(&remote.bucket, &s3_key, version_id.as_deref());
            println!("got remote url: {}", remote_url);

            // "Relax" the manifest by using those new remote keys
            row.place = remote_url.to_string();
        }

        let top_hash = local_manifest.top_hash();
        let new_remote = RemoteManifest {
            hash: top_hash.clone(),
            ..remote.clone()
        };

        // Cache the relaxed manifest
        let cache_path = self
            .domain
            .manifest_cache_path(&new_remote.bucket, &new_remote.hash);

        local_manifest
            .write_to_upath(&UPath::Local(cache_path.clone()))
            .await
            .map_err(|err| err.to_string())?;

        // Push the (cached) relaxed manifest to the remote, don't tag it yet
        let manifest_key = format!(
            "{MANIFEST_DIR}/{}",
            parquet_manifest_filename(&new_remote.hash)
        );
        println!("writing remote manifest to {manifest_key}");

        // TODO: FAIL if the manifest with this hash already exists
        let body = ByteStream::from_path(&cache_path)
            .await
            .map_err(|err| err.to_string())?;
        client
            .put_object()
            .bucket(&new_remote.bucket)
            .key(&manifest_key)
            .body(body)
            .send()
            .await
            .map_err(|err| err.to_string())?;

        // Upload a quilt3 manifest for backward compatibility.
        let quilt3_manifest = Manifest {
            header: ManifestHeader {
                version: "v0".into(),
                message: local_manifest
                    .header
                    .info
                    .get("message")
                    .map(|v| v.as_str())
                    .flatten()
                    .map(|s| s.to_string()),
                user_meta: local_manifest.header.meta.as_object().cloned(),
            },
            rows: local_manifest
                .records
                .values()
                .map(|row| {
                    let mut meta = match row.info.as_object() {
                        Some(meta) => meta.clone(),
                        None => serde_json::Map::default(),
                    };
                    if row.meta.is_object() {
                        meta.insert("user_meta".into(), row.meta.clone());
                    }
                    ManifestRow {
                        logical_key: row.name.clone(),
                        physical_key: row.place.clone(),
                        hash: row.hash.try_into().unwrap(), // TODO: Why doesn't "?" work here???
                        size: row.size,
                        meta: Some(meta),
                    }
                })
                .collect(),
        };
        client
            .put_object()
            .bucket(&new_remote.bucket)
            .key(format!("{MANIFEST_DIR}/{}", &new_remote.hash))
            .body(quilt3_manifest.to_jsonlines().as_bytes().to_vec().into())
            .send()
            .await
            .map_err(|err| err.to_string())?;

        println!("uploaded remote manifest: {new_remote:?}");

        // Tag the new commit.
        // If {self.commit.tag} does not already exist at
        // {self.remote}/.quilt/named_packages/{self.namespace},
        // create it with the value of {self.commit.hash}
        // TODO: Otherwise try again with the current timestamp as the tag
        // (e.g., try five times with exponential backoff, then Error)

        client
            .put_object()
            .bucket(&new_remote.bucket)
            .key(&format!(
                "{TAGS_DIR}/{}/{}",
                new_remote.namespace,
                commit.timestamp.timestamp(),
            ))
            .body(new_remote.hash.as_bytes().to_vec().into())
            .send()
            .await
            .map_err(|err| err.to_string())?;

        // Check the hash of remote's latest manifest
        lineage.latest_hash = new_remote.resolve_latest().await?;
        lineage.remote = new_remote;

        // Reset the commit state.
        lineage.commit = None;

        // Try certifying latest if tracking
        if lineage.base_hash == lineage.latest_hash {
            // remote latest has not been updated, certifying the new latest
            lineage.remote.update_latest(top_hash.clone()).await?;
            lineage.latest_hash = top_hash.clone();
            lineage.base_hash = top_hash.clone();
        }

        self.write_lineage(lineage).await?;

        Ok(())
    }

    pub async fn pull(&self) -> Result<(), String> {
        let status = self.status().await?;
        if !status.changes.is_empty() {
            return Err("package has pending changes".into());
        }

        let lineage = self.lineage().await?;
        if lineage.commit.is_some() {
            return Err("package has pending commits".into());
        }
        if lineage.remote.hash != lineage.base_hash {
            return Err("package is has diverged".into());
        }
        // TODO: do we need to explicity update latest_hash?
        // status() tries to update, but may fail.
        if lineage.base_hash == lineage.latest_hash {
            return Err("package is already up to date".into());
        }

        // TODO: What should we do about installed paths?
        // They may or may not exist in the updated package.
        let paths: Vec<String> = lineage.paths.keys().cloned().collect();
        self.uninstall_paths(&paths).await?;

        // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
        // There needs to be a better way.
        let mut lineage = self.lineage().await?;
        lineage.remote.hash = lineage.latest_hash.clone();
        lineage.base_hash = lineage.latest_hash.clone();

        self.domain.cache_remote_manifest(&lineage.remote).await?;
        tokio::fs::copy(
            self.domain
                .manifest_cache_path(&lineage.remote.bucket, &lineage.remote.hash),
            self.domain
                .installed_manifest_path(&self.namespace, &lineage.remote.hash),
        )
        .await
        .map_err(|err| err.to_string())?;

        self.write_lineage(lineage).await?;

        let manifest = self.manifest().await?.read().await?;
        let paths_to_install = paths
            .into_iter()
            .filter(|x| manifest.records.contains_key(x))
            .collect();
        self.install_paths(&paths_to_install).await?;

        Ok(())
    }

    pub async fn certify_latest(&self) -> Result<(), String> {
        let mut lineage = self.lineage().await?;
        let new_latest = lineage.remote.hash.clone();
        lineage.remote.update_latest(new_latest.clone()).await?;
        lineage.latest_hash = new_latest.clone();
        lineage.base_hash = new_latest;
        self.write_lineage(lineage).await
    }

    pub async fn reset_to_latest(&self) -> Result<(), String> {
        let lineage = self.lineage().await?;

        let new_latest = lineage.remote.resolve_latest().await?;
        if new_latest == lineage.remote.hash {
            // already at latest
            return Ok(());
        }

        let paths: Vec<String> = lineage.paths.into_keys().collect();
        self.uninstall_paths(&paths).await?;
        let mut lineage = self.lineage().await?;

        lineage.latest_hash = new_latest.clone();
        lineage.remote.hash = new_latest.clone();
        lineage.base_hash = new_latest;

        self.domain.cache_remote_manifest(&lineage.remote).await?;
        tokio::fs::copy(
            self.domain
                .manifest_cache_path(&lineage.remote.bucket, &lineage.remote.hash),
            self.domain
                .installed_manifest_path(&self.namespace, &lineage.remote.hash),
        )
        .await
        .map_err(|err| err.to_string())?;

        self.write_lineage(lineage).await?;

        let manifest = self.manifest().await?.read().await?;
        let paths_to_install = paths
            .into_iter()
            .filter(|x| manifest.records.contains_key(x))
            .collect();
        self.install_paths(&paths_to_install).await
    }
}

fn make_s3_url(bucket: &str, s3_key: &str, version_id: Option<&str>) -> Url {
    let mut remote_url = Url::parse("s3://").unwrap();
    remote_url
        .set_host(Some(bucket))
        .expect("failed to set bucket");
    remote_url.set_path(s3_key);
    if let Some(version_id) = version_id {
        remote_url
            .query_pairs_mut()
            .append_pair("versionId", version_id);
    }
    remote_url
}


fn get_compatible_chunked_checksum(attrs: &aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput) -> Option<Vec<u8>> {
    // TODO: get checksums from multipart objects
    let checksum = attrs.checksum.as_ref()?;
    let checksum_sha256 = checksum.checksum_sha256.as_ref()?;
    // XXX: defer decoding until we know it's compatible?
    let checksum_sha256_decoded = BASE64_STANDARD.decode(checksum_sha256.as_bytes())
        .expect("AWS checksum must be valid base64");
    let object_size = attrs.object_size.expect("ObjectSize must be requested");
    if (object_size as u64) < MULTIPART_THRESHOLD {
        if let Some(object_parts) = &attrs.object_parts {
            if object_parts.total_parts_count.expect("ObjectParts is expected to have TotalParts") == 1 {
                return Some(checksum_sha256_decoded);
            }
        }
        return Some(Sha256::digest(checksum_sha256_decoded).as_slice().into());
    }
    None
}

pub async fn package_s3_prefix(uri: &s3::S3Uri) -> Result<Table, String> {
    // TODO: TODOs in .expect()
    // TODO: make get_object_attributes() calls concurrently
    // XXX: validate prefix
    let client = crate::s3_utils::get_client_for_bucket(&uri.bucket)
        .await
        .expect("TODO");

    // FIXME: we need real API to build manifests
    let header = Row4 {
        name: HEADER_ROW.into(),
        place: HEADER_ROW.into(),
        path: None,
        size: 0,
        hash: Multihash::default(),
        info: serde_json::json!({
            "message": "TODO: ???",
            "version": "TODO: ???",
        }),
        meta: serde_json::Value::Null, // TODO: accept user meta?
    };
    let mut records: BTreeMap<String, Row4> = BTreeMap::new();

    let prefix_len = uri.key.len();
    let mut p = client
        .list_objects_v2()
        .bucket(&uri.bucket)
        .prefix(&uri.key)
        .into_paginator()
        .send();
    while let Some(page) = p.next().await {
        let page = page.expect("TODO");
        println!("PAGE {:?}", page);
        for obj in page.contents.as_ref().expect("TODO") {
            dbg!(obj);
            let key = obj.key.as_ref().expect("TODO");
            let attrs = client
                .get_object_attributes()
                .bucket(&uri.bucket)
                .key(key)
                .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
                .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
                .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
                .max_parts(10_000) // TODO: use const
                .send()
                .await
                .expect("TODO");
            dbg!(&attrs);
            if attrs.delete_marker.is_some() && attrs.delete_marker.expect("TODO") {
                // XXX: do something different?
                continue;
            }
            let name = &key[prefix_len..];
            // FIXME: we assume that objects have hash and it's compatible with sha-256-chunked
            let s3_checksum = get_compatible_chunked_checksum(&attrs);
            let hash =
                Multihash::wrap(MULTIHASH_SHA256_CHUNKED, s3_checksum.unwrap().as_bytes()).unwrap();
            records.insert(
                name.into(),
                Row4 {
                    name: name.into(),
                    place: make_s3_url(&uri.bucket, &key, attrs.version_id.as_deref()).into(),
                    path: None, // WTF is this?
                    // This shouldn't be empty because we requested it
                    // XXX: can we use `as u64` safely here?
                    size: attrs.object_size.expect("TODO") as u64,
                    hash,
                    info: serde_json::Value::Null, // XXX: is this right?
                    meta: serde_json::Value::Null, // XXX: is this right?
                },
            );
            dbg!(name);
        }
    }

    dbg!(&records);

    Ok(Table { header, records })
}

// a conflict is identified by the identifiers of the two conflicting manifests
#[derive(Debug, PartialEq)]
pub struct Conflict {
    package: InstalledPackage,
    changes: ChangeSet<String, PackageFileFingerprint>,
    folder: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use temp_testdir::TempDir;
    use tokio_test::{assert_err, block_on};

    fn get_timestamp() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }

    #[test]
    #[ignore]
    fn flow() {
        // ## Setup
        let test_uri_string = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";

        let test_uri = S3PackageURI::try_from(test_uri_string).expect("Failed to parse URI");
        assert_eq!(
            test_uri,
            S3PackageURI {
                bucket: "udp-spec".into(),
                namespace: "spec/quiltcore".into(),
                path: Some("READ ME.md".into()),
                revision: RevisionPointer::default(),
            }
        );

        // TODO: abstract s3 and fs, inject mocked versions
        // let root_fs = fs::MemoryFS::from_strs([("/", "")]);
        //
        // let stage_bucket = fs::MemoryFS::from_strs([("key", "contents")]);

        // let buckets = s3::MemoryBuckets::from([(String::from("quilt-example"), stage_bucket)]);

        // let context = TestContext::new(&root_fs, &buckets);

        // let local_path = UPath(scheme: "file", path: "$HOME/Documents/QuiltSync")
        // let local_path = "/home/quilt-sync";
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        // ## Pull the manifest

        let remote_manifest =
            block_on(RemoteManifest::resolve(&test_uri)).expect("Failed to resolve manifest");

        let cached_manifest = block_on(local_domain.cache_remote_manifest(&remote_manifest))
            .expect("Failed to cache the manifest");

        let manifest = block_on(cached_manifest.read()).expect("Failed to parse the manifest");

        println!("manifest: {manifest:?}");
        // TODO: assert manifest has the expected contents

        // ## Install the files
        //
        // installed_package knows its domain and install folder
        // downloads manifest for latest revision into an editable working copy
        // creates install folder and Lineages it to remote_package_uri

        let paths = vec![test_uri.path.unwrap()];
        let installed_package = block_on(local_domain.install_package(&remote_manifest))
            .expect("Failed to install package");
        block_on(installed_package.install_paths(&paths)).expect("Failed to install paths");

        // Can only install it once.
        assert_err!(block_on(installed_package.install_paths(&paths)));

        // TODO: assert files are installed

        // List filenames in Manifest
        // XXX: does this list the files in the working directory or in the manifest or a
        // combination of those?
        // for entry in installed_package.entries() {
        //     println!("{:?}", entry.logical_key);
        // }

        let status = block_on(installed_package.status()).expect("Failed to get status");
        assert_eq!(status, InstalledPackageStatus::default());

        // ## Modify installed files

        let readme_path = installed_package.working_folder().join("READ ME.md");
        println!("readme_path: {readme_path:?}");

        let old_readme =
            block_on(fs::read_to_string(&readme_path)).expect("Failed to read 'READ ME.md'");

        let timestamp = get_timestamp();
        println!("timestamp: {timestamp:?}");
        block_on(fs::write(readme_path, timestamp.as_bytes()))
            .expect("Failed to overwrite 'READ ME.md'");
        let status = block_on(installed_package.status()).expect("Failed to get status");
        let expected_status = InstalledPackageStatus::new(
            UpstreamState::default(),
            ChangeSet::from([(
                "READ ME.md".into(),
                Change {
                    current: Some(PackageFileFingerprint {
                        size: timestamp.len() as u64,
                        hash: Multihash::wrap(
                            MULTIHASH_SHA256,
                            block_on(calculate_sha256_checksum(timestamp.as_bytes()))
                                .unwrap()
                                .as_ref(),
                        )
                        .unwrap(),
                    }),
                    previous: Some(PackageFileFingerprint {
                        size: old_readme.len() as u64,
                        hash: Multihash::wrap(
                            MULTIHASH_SHA256,
                            block_on(calculate_sha256_checksum(old_readme.as_bytes()))
                                .unwrap()
                                .as_ref(),
                        )
                        .unwrap(),
                    }),
                },
            )]),
        );
        assert_eq!(status, expected_status);

        // ## Commit

        let commit_message = format!("Commit made at {}", timestamp);
        let user_meta = serde_json::json!({
            "test": "value",
            "timestamp": timestamp,
        })
        .as_object()
        .unwrap()
        .to_owned();

        // commit local state to the local manifest,
        // accumulate commit metadata in lineage
        block_on(installed_package.commit(commit_message, Some(user_meta)))
            .expect("Failed to commit");

        // // ## Sync
        // let remote_latest = block_on(installed_package.push())
        //     .expect("Failed to push")
        //     .expect("Expected to return diverged remote latest");
        //
        // // TODO: inject new remote latest
        // let expected_remote_latest = RemoteManifest {
        //     bucket: "quilt-example".into(),
        //     namespace: "akarve/test_dest".into(),
        //     hash: "abc".into(),
        // };
        //
        // assert_eq!(remote_latest, expected_remote_latest);
        //
        // // ## Merge
        // // latest not certified -- merging
        //
        // let merge = block_on(installed_package.merge(remote_latest)).expect("Failed to start merge");
        //
        // // open folder
        // // Tell user to compare with working versions
        // // resolve conflicts
        // // Once the user confirms they have resolved merge conflicts:
        //
        // block_on(merge.commence()).expect("Failed to commence merge");
        //
        // block_on(installed_package.commit("merge", user_meta)).expect("Failed to commit merge");
        //
        // let remote_latest = block_on(installed_package.push()).expect("Failed to push");
        //
        // assert_eq!(remote_latest, None, "Expected to certify remote latest");
    }
}

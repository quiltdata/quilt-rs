use std::path::PathBuf;
use std::path::StripPrefixError;
use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;

use crate::io::remote::HostChecksums;
use crate::object_hash::Error as ObjectHashError;
use crate::workflow::WorkflowValidationError;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::UriError;

#[derive(Error, Debug)]
#[error("S3 error{}: {kind}", .host.as_ref().map_or(String::new(), |h| format!(" for {h}")))]
pub struct S3Error {
    pub host: Option<Host>,
    #[source]
    pub kind: S3ErrorKind,
}

impl S3Error {
    #[must_use]
    pub fn new(kind: S3ErrorKind) -> Self {
        Self { host: None, kind }
    }

    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self.kind, S3ErrorKind::NotFound(_))
    }

    /// True when S3 refused the call with the `AccessDenied` code — under a
    /// vended credential that proves the session is healthy, this means the
    /// active role cannot reach the object, not that the user is logged out.
    #[must_use]
    pub fn is_access_denied(&self) -> bool {
        matches!(self.kind, S3ErrorKind::AccessDenied(_))
    }

    /// True when the S3 service rejected the credentials themselves rather
    /// than the active role's access to an object.
    #[must_use]
    pub fn is_invalid_credentials(&self) -> bool {
        matches!(self.kind, S3ErrorKind::InvalidCredentials(_))
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum S3ErrorKind {
    #[error("Failed to check object existence: {0}")]
    Exists(String),

    #[error("Failed to get object: {0}")]
    GetObject(String),

    #[error("Failed to get object attributes: {0}")]
    GetObjectAttributes(String),

    #[error("Failed to get object stream: {0}")]
    GetObjectStream(String),

    #[error("Failed to initialize S3 client: {0}")]
    Client(String),

    #[error("Failed to list objects: {0}")]
    ListObjects(String),

    #[error("Failed to put object: {0}")]
    PutObject(String),

    #[error("Failed to resolve object URL: {0}")]
    ResolveUrl(String),

    #[error("Failed to upload object: {0}")]
    UploadFile(String),

    #[error("S3 not found: {0}")]
    NotFound(String),

    #[error("S3 access denied: {0}")]
    AccessDenied(String),

    #[error("Invalid AWS credentials: {0}")]
    InvalidCredentials(String),

    #[error("S3 error: {0}")]
    Raw(String),

    #[error("Object key expected to be present")]
    ObjectKey,

    #[error("Error with upload id: {0}")]
    UploadId(String),

    #[error("Failed to read RwLock: {0}")]
    PoisonLock(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum AuthError {
    #[error("Failed to read credentials: {0}")]
    CredentialsRead(String),

    #[error("Failed to refresh credentials: {0}")]
    CredentialsRefresh(String),

    #[error("Failed to read tokens: {0}")]
    TokensRead(String),

    #[error("Failed to refresh tokens: {0}")]
    TokensRefresh(String),

    #[error("Failed to exchange authorization code for tokens: {0}")]
    TokensExchange(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum RoleError {
    #[error("Not authenticated with {0}")]
    NotAuthenticated(Host),

    #[error("Registry rejected the request: {0}")]
    GraphQl(String),

    #[error("Role switch rejected: {0}")]
    SwitchRejected(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum InstallPackageError {
    #[error("The package {0} is already installed")]
    AlreadyInstalled(Namespace),

    #[error("The given package is not installed: {0}")]
    NotInstalled(Namespace),
}

#[derive(Error, Debug, PartialEq)]
pub enum InstallPathError {
    #[error("Failed to install path: {}", .0.display())]
    Install(PathBuf),

    /// The path has no row in the revision being installed. Reachable when a
    /// download waited out a pull that moved the package to a revision
    /// without it.
    #[error("{} is not in the package's current revision", .0.display())]
    NotInRevision(PathBuf),

    /// Two keys of the package name one local file, or the first key's file
    /// would be a folder of the second's. Windows reads a `\` in a key as a
    /// folder separator, so `a/b` and `a\b` are one file there, and `data`
    /// can't be a file and the folder of `data\x.csv`.
    #[error(
        "'{0}' and '{1}' can't both be installed on this system: they name the same local file, or the first is a folder of the second"
    )]
    LocalPathClash(String, String),

    #[error(
        "A local file is already at {}",
        .0.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    LocalFileExists(Vec<PathBuf>),

    /// The remote returned bytes that do not hash to a row whose
    /// `physical_key` carries no `versionId`: it names whatever the object is
    /// now, and a later revision has replaced it, so this revision's content
    /// is no longer on the remote.
    ///
    /// Raised only where a partial apply would be wrong, the pull's reconcile.
    /// `install_paths` skips such a row and reports it instead.
    #[error(
        "The remote no longer holds this revision's content for {}: the object it points to has been replaced",
        .0.display()
    )]
    ContentMismatch(PathBuf),

    /// The remote returned bytes that do not hash to a row whose
    /// `physical_key` carries a `versionId`. That version's bytes cannot have
    /// changed, so the hash the row records is wrong.
    ///
    /// Raised only where a partial apply would be wrong, the pull's reconcile.
    /// `install_paths` leaves such a row out and reports it instead.
    #[error(
        "The package records a hash for {} that its file does not match: the package may be corrupt",
        .0.display()
    )]
    HashMismatch(PathBuf),

    #[error("Failed to uninstall path: {}", .0.display())]
    Uninstall(PathBuf),
}

/// The message of a hash mismatch on read, in the words `install_paths` uses.
fn hash_mismatch_message(key: &str, replaced: bool, expected: &str, computed: &str) -> String {
    if replaced {
        format!(
            "The remote no longer holds this revision's content for {key}: the object it points to has been replaced (expected {expected}, got {computed})"
        )
    } else {
        format!(
            "The package records a hash for {key} that its file does not match: the package may be corrupt (expected {expected}, got {computed})"
        )
    }
}

/// The message of a size mismatch on read.
fn size_mismatch_message(key: &str, replaced: bool, expected: u64, actual: u64) -> String {
    let message = format!("{key} is {actual} bytes, but its row says {expected}");
    if replaced {
        format!("{message}: the object it points to may have been replaced")
    } else {
        message
    }
}

#[derive(Error, Debug)]
pub enum ChecksumError {
    #[error("Checksum error: {0}")]
    Mismatch(String),

    /// A read object's bytes do not hash to its row. `replaced` is whether the
    /// row's `physical_key` has no `versionId` (or the `null` one), so a later
    /// revision may have replaced the object: the wording is the one
    /// `install_paths` uses for that case, and for a pinned version, whose bytes
    /// cannot have changed and so whose recorded hash must be wrong.
    #[error("{}", hash_mismatch_message(.key, *.replaced, .expected, .computed))]
    HashMismatch {
        /// The row's logical key.
        key: String,
        /// The hash the row records.
        expected: String,
        /// The hash of the bytes that were read.
        computed: String,
        /// Whether the object may have been replaced since the row was made.
        replaced: bool,
    },

    /// A read object is not the size its row records. `replaced` is as for
    /// [`HashMismatch`](Self::HashMismatch).
    #[error("{}", size_mismatch_message(.key, *.replaced, *.expected, *.actual))]
    SizeMismatch {
        /// The row's logical key.
        key: String,
        /// The size the row records.
        expected: u64,
        /// How many bytes were read.
        actual: u64,
        /// Whether the object may have been replaced since the row was made.
        replaced: bool,
    },

    #[error("Missing checksum: {0:?}")]
    Missing(HostChecksums),

    #[error("Malformed checksum: {0}")]
    Malformed(String),

    #[error("Failed to get checksum from S3: {0}")]
    NoS3Checksum(String),
}

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Manifest header: {0}")]
    Header(String),

    #[error("Failed to load manifest from {path}: {source}")]
    Load {
        path: PathBuf,
        source: Box<crate::Error>,
    },

    #[error("Table error: {0}")]
    Table(String),

    #[error("Manifest lists the logical key {0:?} more than once")]
    DuplicateLogicalKey(String),

    #[error("Manifest lists {0:?} both as a file and as a folder")]
    FileAndFolder(String),
}

#[derive(Error, Debug)]
pub enum LineageError {
    #[error("Domain lineage missing, including missing Home directory")]
    Missing,

    #[error("Domain lineage missing Home directory")]
    MissingHome,

    #[error("Failed to parse lineage file: {0}")]
    Parse(serde_json::Error),

    #[error("Operation requires a remote origin, but this is a local-only package")]
    NoRemote,

    /// The home can't move while packages are installed: their working
    /// folders live under the current one.
    #[error(
        "Cannot change the home: {installed} {} installed in {}",
        if *installed == 1 { "package is" } else { "packages are" },
        home.display()
    )]
    HomeInUse { home: PathBuf, installed: usize },

    /// A folder to keep reaches outside the package through `..`.
    #[error(
        "Folder {0:?} is not inside the package: name it from the package root, without \"..\""
    )]
    FolderOutsidePackage(String),

    /// A package's ref or the domain config does not parse.
    #[error("Failed to parse {}: {source}", path.display())]
    FileParse {
        path: PathBuf,
        source: serde_json::Error,
    },

    /// A file written in a format this version does not read, most likely by
    /// a newer one, which may have added fields this version would drop.
    #[error(
        "{} is in format {found}, and this version of quilt reads format {supported}",
        path.display()
    )]
    UnsupportedFormat {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    #[error(
        "The home changed from {} to {} while the package was being placed, so it was not recorded",
        placed.display(),
        home.display()
    )]
    HomeChanged { placed: PathBuf, home: PathBuf },

    /// The domain config, at `path`, does not say the domain's lineage has
    /// moved to refs, so no ref is written.
    #[error(
        "Lineage cannot be written while the move to per-package refs {state} ({})",
        path.display()
    )]
    RefsNotWritable { path: PathBuf, state: &'static str },
}

#[derive(Error, Debug, PartialEq)]
pub enum RemoteCatalogError {
    #[error("Workflow error: {0}")]
    Workflow(String),

    /// The bucket's `.quilt/workflows/config.yml` is malformed — it violates the
    /// vendored quilt3 config schema, or its YAML could not be converted for
    /// validation. Distinct from [`RemoteCatalogError::Workflow`] (a workflow
    /// that resolved against a *valid* config could not be found/applied): an
    /// invalid config means every commit to the bucket will fail until it is
    /// fixed, so callers can present that honestly instead of "couldn't load".
    #[error("Invalid workflows config: {0}")]
    InvalidWorkflowsConfig(String),

    #[error("Failed to fetch host config: {0}")]
    HostConfig(String),

    #[error("S3 bucket '{0}' is not reachable — verify the bucket name")]
    BucketUnreachable(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum LoginError {
    /// No usable session for this deployment; `None` when the call carried no
    /// deployment — a bare bucket on ambient AWS credentials.
    ///
    /// Names the state, not a remedy: each surface answers it differently.
    #[error("No session{}", .0.as_ref().map_or(String::new(), |h| format!(" for {h}")))]
    NoSession(Option<Host>),

    /// The deployment answered but its `config.json` names no registry. A
    /// misconfiguration, not a missing session: signing in cannot change it.
    #[error("{0} does not advertise a registry URL in its config.json")]
    NoRegistryUrl(Host),
}

#[derive(Error, Debug)]
pub enum FsError {
    #[error("Failed to read file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to write file {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to copy file from {from} to {to}: {source}")]
    Copy {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to create directory {path}: {source}")]
    DirectoryCreate {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("File not found: {path}")]
    NotFound { path: PathBuf },

    #[error("Path prefix not found: {0}")]
    PathPrefixNotFound(StripPrefixError),

    #[error("ByteStream error: {0}")]
    ByteStream(#[from] byte_stream::error::Error),
}

/// Why a short top hash did not resolve to one revision.
///
/// The three outcomes quilt3's `resolve_top_hash` tells apart
/// (`api/python/quilt3/backends/base.py` in quiltdata/quilt): a prefix of the
/// wrong length, a prefix no revision starts with, and one that several do;
/// plus [`NotHex`](Self::NotHex), which quilt3 has no case for.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum HashPrefixError {
    /// Shorter than [`MIN_HASH_PREFIX_LEN`](crate::flow::MIN_HASH_PREFIX_LEN) or
    /// longer than a whole top hash.
    #[error(
        "Invalid hash: {0:?} (a hash prefix is {min} to {max} characters)",
        min = crate::flow::MIN_HASH_PREFIX_LEN,
        max = crate::flow::TOP_HASH_LEN
    )]
    InvalidLength(String),

    /// Not lowercase hex, so no top hash starts with it. Refused without
    /// asking the registry.
    #[error("Invalid hash: {0:?} (a hash is lowercase hex)")]
    NotHex(String),

    /// No revision in the registry starts with the prefix.
    #[error("Found zero matches for {0:?}")]
    NoMatch(String),

    /// More than one revision starts with the prefix. `matches` lists the
    /// first [`MAX_LISTED_MATCHES`](crate::flow::MAX_LISTED_MATCHES), sorted,
    /// so the caller can say which to pick, and `more` counts the rest.
    #[error(
        "Found multiple matches: {prefix:?} starts {} revisions ({}{})",
        .matches.len() + .more,
        .matches.join(", "),
        and_more(*.more)
    )]
    Ambiguous {
        prefix: String,
        matches: Vec<String>,
        more: usize,
    },
}

fn and_more(more: usize) -> String {
    if more == 0 {
        String::new()
    } else {
        format!(", and {more} more")
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum PackageOpError {
    #[error("Commit error: {0}")]
    Commit(String),

    #[error("Push error: {0}")]
    Push(String),

    /// The revision's manifest was stored, but writing its timestamp tag, or
    /// reading or writing `latest`, failed. The revision is reachable by
    /// `top_hash`; whether `latest` moved is unknown. `reason` is the failure.
    #[error(
        "Pushed revision {top_hash}, but could not finish publishing it: {reason}. The revision manifest is saved; latest may not have been updated."
    )]
    PushTagFailed {
        /// The top hash of the revision whose manifest was stored.
        top_hash: String,
        /// What failed, as the failing call reported it.
        reason: String,
    },

    /// A push whose parent is no longer `latest`: the revision was written,
    /// and `latest` was left alone.
    #[error(
        "Pushed revision {top_hash}, but did not make it latest: latest is {latest}, but this push was built on {expected}. The revision is saved, and latest is unchanged."
    )]
    PushedNotLatest {
        /// The top hash of the revision that was written.
        top_hash: String,
        /// The revision `latest` still names.
        latest: String,
        /// What the push was built on: "revision <hash>", or "no earlier revision".
        expected: String,
    },

    #[error("Publish error: {0}")]
    Publish(String),

    #[error("Undo error: {0}")]
    Undo(String),

    #[error("General error regarding package: {0}")]
    Package(String),

    #[error("Pull blocked by conflicting local changes: {0:?}")]
    PullConflict(Vec<PathBuf>),

    /// The package tracks files, but its working folder is missing or cannot be
    /// read, so what the files hold, or whether they were deleted, is unknown.
    /// Restore the folder, or uninstall the package.
    #[error(
        "The working folder {} is missing or cannot be read, so the package's files cannot be checked; restore it, or uninstall the package",
        .0.display()
    )]
    WorkingCopyUnavailable(PathBuf),
}

/// The error type for this library
#[derive(Error, Debug)]
pub enum Error {
    #[error("Authentication failed for {0}: {1}")]
    Auth(Host, AuthError),

    #[error(transparent)]
    Checksum(#[from] ChecksumError),

    #[error(transparent)]
    Fs(#[from] FsError),

    #[error(transparent)]
    HashPrefix(#[from] HashPrefixError),

    #[error(transparent)]
    InstallPackage(InstallPackageError),

    #[error(transparent)]
    InstallPath(InstallPathError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Lineage(#[from] LineageError),

    #[error(transparent)]
    Login(#[from] LoginError),

    #[error(transparent)]
    LogicalKey(#[from] crate::LogicalKeyError),

    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error(transparent)]
    ObjectHash(#[from] ObjectHashError),

    #[error(transparent)]
    PackageOp(#[from] PackageOpError),

    /// Another writer holds this package's lock, so a domain-wide sweep
    /// that needs every package still would be unsafe to start.
    #[error("{0} is busy in another quilt process; try again once it finishes")]
    PackageBusy(Namespace),

    /// A pruning uninstall removed the package, then failed before it had
    /// deleted every object it meant to: it could not prove them unused, so
    /// deleted none, or a deletion failed part way.
    /// The third field is a package that was busy as well, which kept the
    /// files that could be counted.
    #[error(
        "Uninstalled {0}, but not all of its downloaded files were deleted: {1}{busy}",
        busy = kept_by_busy(.2.as_ref())
    )]
    PruneFailed(Namespace, Box<Error>, Option<Namespace>),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// A revision asked to be removed is protected, or this copy does not
    /// hold it. Nothing was removed.
    #[error("Can't remove revision {hash} of {namespace}: {why}")]
    RevisionNotRemovable {
        namespace: Namespace,
        hash: String,
        why: String,
    },

    #[error(transparent)]
    Role(#[from] RoleError),

    #[error(transparent)]
    RemoteCatalog(#[from] RemoteCatalogError),

    #[error(transparent)]
    S3(#[from] S3Error),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

    #[error("Integer conversion error: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    #[error("Unimplemented")]
    Unimplemented,

    #[error(transparent)]
    Uri(#[from] UriError),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),

    #[error(transparent)]
    WorkflowValidation(#[from] WorkflowValidationError),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_saphyr::Error),
}

/// Map a workflow config error onto the existing `Error` variants so the
/// consumer-visible `Display` and variant matching are byte-identical to before
/// the crate extraction. The mapping is cross-variant (a single `ConfigError`
/// splits across `RemoteCatalog` and `Uri`), so it cannot be a `#[from]` field
/// attribute.
impl From<crate::workflow::ConfigError> for Error {
    fn from(err: crate::workflow::ConfigError) -> Self {
        use crate::workflow::ConfigError;
        match err {
            ConfigError::Workflow(msg) => Error::RemoteCatalog(RemoteCatalogError::Workflow(msg)),
            ConfigError::InvalidWorkflowsConfig(msg) => {
                Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(msg))
            }
            ConfigError::Uri(err) => Error::Uri(err),
        }
    }
}

impl Error {
    /// Returns `true` if this error represents a "not found" — an S3
    /// `NoSuchKey` response, or a local filesystem miss (a working-tree file
    /// that was deleted). Lets callers tell an absent object/file apart from a
    /// genuine I/O failure (permission denied, transient storage error).
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        match self {
            Error::S3(s3) => s3.is_not_found(),
            Error::Fs(FsError::NotFound { .. }) => true,
            Error::Fs(FsError::Read { source, .. }) => {
                source.kind() == std::io::ErrorKind::NotFound
            }
            Error::Io(e) => e.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }

    /// Returns `true` if S3 refused the call with the `AccessDenied` code.
    ///
    /// The mirror of [`Error::is_not_found`] for the role dimension: callers
    /// that only ever see an [`enum@Error`] need this to tell "the active role
    /// cannot reach this object" apart from a generic storage failure, so
    /// every layer that re-wraps an S3 error must let this variant through
    /// unchanged.
    #[must_use]
    pub fn is_access_denied(&self) -> bool {
        matches!(self, Error::S3(s3) if s3.is_access_denied())
    }

    /// Returns `true` if S3 rejected the credentials themselves.
    ///
    /// Distinct from [`Error::is_access_denied`]: that one means the session is
    /// healthy and the active role cannot reach the object, this one means the
    /// session is not healthy. Callers that back off on storage failures must
    /// check this first — retrying a rejected credential never succeeds.
    #[must_use]
    pub fn is_invalid_credentials(&self) -> bool {
        matches!(self, Error::S3(s3) if s3.is_invalid_credentials())
    }

    /// No usable session, by either route — refused before issue
    /// ([`LoginError::NoSession`]) or issued and rejected
    /// ([`S3ErrorKind::InvalidCredentials`]). One predicate because callers act
    /// on the state; which route it took is where the failure was noticed.
    #[must_use]
    pub fn is_session_absent(&self) -> bool {
        self.is_invalid_credentials() || matches!(self, Error::Login(LoginError::NoSession(_)))
    }

    /// No usable session and no deployment to sign in to: the AWS credentials
    /// on this computer, used for a bucket with no catalog, were missing or
    /// refused. The remedy is `~/.aws` or `aws sso login`, not a sign-in.
    #[must_use]
    pub fn is_ambient_session_absent(&self) -> bool {
        match self {
            Error::Login(LoginError::NoSession(host)) => host.is_none(),
            Error::S3(s3) => s3.is_invalid_credentials() && s3.host.is_none(),
            _ => false,
        }
    }

    /// The deployment this request was for, if any.
    ///
    /// `None` for a bare S3 bucket reached with ambient credentials, and for
    /// errors that are not S3 errors. What that absence *means* is the
    /// caller's to decide: paired with [`Error::is_invalid_credentials`] it
    /// says there is no stack to sign back in to.
    #[must_use]
    pub fn s3_host(&self) -> Option<&quilt_uri::Host> {
        match self {
            Error::S3(s3) => s3.host.as_ref(),
            _ => None,
        }
    }
}

// Compose `?` across two From hops: external error → focused enum → Error.
// Rust's `?` only runs one `From::from`, so these bridges make call sites
// keep working without `.map_err(..)`.

impl From<multihash::Error> for Error {
    fn from(err: multihash::Error) -> Self {
        Error::ObjectHash(err.into())
    }
}

impl From<multibase::Error> for Error {
    fn from(err: multibase::Error) -> Self {
        Error::ObjectHash(err.into())
    }
}

impl From<byte_stream::error::Error> for Error {
    fn from(err: byte_stream::error::Error) -> Self {
        FsError::ByteStream(err).into()
    }
}

impl From<StripPrefixError> for Error {
    fn from(err: StripPrefixError) -> Self {
        Error::Fs(FsError::PathPrefixNotFound(err))
    }
}

/// The tail [`Error::PruneFailed`] adds when a busy package kept the rest.
fn kept_by_busy(busy: Option<&Namespace>) -> String {
    busy.map_or_default(|namespace| format!("; {namespace} is busy, so the rest were kept too"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_uri::fixtures;

    #[test]
    fn an_ambient_session_is_absent_only_without_a_host() {
        let invalid = |host| {
            Error::S3(S3Error {
                host,
                kind: S3ErrorKind::InvalidCredentials("refused".to_string()),
            })
        };
        assert!(Error::Login(LoginError::NoSession(None)).is_ambient_session_absent());
        assert!(invalid(None).is_ambient_session_absent());

        assert!(
            !Error::Login(LoginError::NoSession(Some(fixtures::host())))
                .is_ambient_session_absent()
        );
        assert!(!invalid(Some(fixtures::host())).is_ambient_session_absent());
        assert!(
            !Error::S3(S3Error::new(S3ErrorKind::AccessDenied("no".to_string())))
                .is_ambient_session_absent()
        );
    }

    #[test]
    fn a_changed_home_names_both_folders() {
        let err = LineageError::HomeChanged {
            placed: PathBuf::from("/old/home"),
            home: PathBuf::from("/new/home"),
        };
        assert_eq!(
            err.to_string(),
            "The home changed from /old/home to /new/home while the package was being placed, so it was not recorded"
        );
    }
}

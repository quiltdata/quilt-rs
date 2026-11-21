//! Not a part of the library and meant to be an independent project.
//! This is a CLI frontend for `quilt_rs`.

use std::path::Path;
use std::path::PathBuf;

use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;
use clap::Subcommand;
use clap::builder::TypedValueParser as _;
use tracing::log;

use quilt_rs::dataspace::certify_latest::CertifyError;
use quilt_rs::dataspace::commit::CommitError;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::ContractError;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::create::CreateError;
use quilt_rs::dataspace::history::HistoryError;
use quilt_rs::dataspace::home::HomeError;
use quilt_rs::dataspace::install::InstallError;
use quilt_rs::dataspace::keep::KeepError;
use quilt_rs::dataspace::lock::LockError;
use quilt_rs::dataspace::publish_all::PublishAllError;
use quilt_rs::dataspace::read_all::ReadAllError;
use quilt_rs::dataspace::reattach::ReattachError;
use quilt_rs::dataspace::remove_revisions::RemoveError;
use quilt_rs::dataspace::rename::RenameError;
use quilt_rs::dataspace::reset_to_latest::ResetError;
use quilt_rs::dataspace::status::StatusError;
use quilt_rs::dataspace::sync::SyncError;
use quilt_rs::dataspace::undo_commit::UndoError;
use quilt_rs::dataspace::uninstall::UninstallError;
use quilt_rs::domain::DOMAIN_DIR_NAME;
use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::WorkflowIntent;
use quilt_rs::logging::InvalidLogFilter;
use quilt_rs::queue::QueueError;
use quilt_uri::Host;
use quilt_uri::Namespace;

mod browse;
mod catalog;
mod certify;
mod commit;
mod create;
mod dataspace;
mod gc;
mod history;
mod home;
mod install;
mod keep;
mod lineage;
mod list;
mod local_registry;
mod login;
mod model;
mod output;
mod publish;
mod pull;
mod push;
mod read;
mod reattach;
mod remove_revisions;
mod reset;
mod role;
mod status;
mod storage;
pub mod switch;
mod sync;
mod text;
mod undo_commit;
mod uninstall;
mod upgrade;
mod version;

#[cfg(test)]
mod fixtures;

use model::Commands as _;
use model::Model;
pub use output::Format;
pub use output::Std;
pub use output::print;

const DOMAIN_DIR_NAMESPACE: &str = "com.quiltdata.quilt-sync";

/// The environment variable `--domain` falls back to.
const DOMAIN_ENV: &str = "QUILT_DOMAIN";

/// The environment variable `--format` falls back to.
const FORMAT_ENV: &str = "QUILT_FORMAT";

/// The values `--format` accepts. The empty one is hidden from `--help`: it is
/// accepted only so `try_parse_with_env` can tell an empty variable (unset)
/// from an empty flag (an error).
fn format_value_parser() -> impl clap::builder::TypedValueParser<Value = String> {
    clap::builder::PossibleValuesParser::new([
        clap::builder::PossibleValue::new("text"),
        clap::builder::PossibleValue::new("json"),
        clap::builder::PossibleValue::new("").hide(true),
    ])
}

/// The `tracing` directives for a `QUILT_LOG` or `--log` value under the
/// shared rule, or `Ok(None)` for an empty value.
///
/// Parsed strictly, so `quilt_rs=lots` is an error rather than being dropped.
pub fn log_filter(value: &str) -> Result<Option<String>, InvalidLogFilter> {
    let parsed = quilt_rs::logging::directives(value)?;
    if let Some(parsed) = &parsed {
        tracing_subscriber::EnvFilter::builder()
            .parse(parsed)
            .map_err(|_| InvalidLogFilter::new(value))?;
    }
    Ok(parsed)
}

/// `--log`'s value parser: an empty value is an error here, unlike an empty
/// `QUILT_LOG`, since it usually means an unset shell variable.
fn parse_log_flag(value: &str) -> Result<String, InvalidLogFilter> {
    log_filter(value)?.ok_or_else(|| InvalidLogFilter::new(value))
}

/// Resolve the commit command's `(--workflow, --no-workflow)` flag pair into a
/// [`WorkflowIntent`] at the clap boundary.
///
/// * `--no-workflow` → [`WorkflowIntent::NoWorkflow`] (explicit opt-out).
/// * an explicitly-passed but blank `--workflow` (empty or whitespace) → error:
///   a scripted unset-variable footgun is rejected loudly rather than silently
///   falling back to the bucket default.
/// * otherwise the id is normalized by [`WorkflowIntent::from_optional_id`]:
///   omitting `--workflow` → [`WorkflowIntent::BucketDefault`], a non-blank id →
///   [`WorkflowIntent::Named`] (trimmed).
fn commit_workflow_intent(
    workflow: Option<&str>,
    no_workflow: bool,
) -> Result<WorkflowIntent, Error> {
    if no_workflow {
        Ok(WorkflowIntent::NoWorkflow)
    } else if matches!(workflow, Some(id) if id.trim().is_empty()) {
        Err(Error::WorkflowEmpty)
    } else {
        Ok(WorkflowIntent::from_optional_id(workflow))
    }
}

/// A package's URI, checked as clap reads it but kept as typed: a
/// `PackageKey` in every command that takes one would grow [`Commands`], and
/// with it the future of [`init`], past the stack budget on Windows. The
/// command parses it again with [`dataspace::Package::uri`].
fn package_uri(text: &str) -> Result<String, quilt_uri::UriError> {
    text.parse::<PackageKey>().map(|_| text.to_string())
}

fn parse_optional_namespace(namespace: Option<String>) -> Result<Option<Namespace>, Error> {
    Ok(match namespace {
        Some(namespace) => Some(namespace.try_into()?),
        None => None,
    })
}

/// The remote that signs in with the credentials the application's folder
/// `app` keeps.
fn app_remote(app: &std::path::Path) -> quilt_rs::io::remote::RemoteS3 {
    quilt_rs::io::remote::RemoteS3::new(
        quilt_rs::paths::DomainPaths::new(app.to_path_buf()),
        quilt_rs::io::storage::LocalStorage::new(),
    )
}

fn get_domain_dir(dir_arg: Option<PathBuf>) -> Result<PathBuf, Error> {
    match dir_arg {
        Some(user_specified_dir) => Ok(user_specified_dir),
        None => default_domain_dir().ok_or(Error::Domain),
    }
}

/// The domain used when neither `--domain` nor `QUILT_DOMAIN` names one.
fn default_domain_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join(DOMAIN_DIR_NAMESPACE))
}

/// The Domain folder that keeps a dataspace's local commits. `--domain` and
/// `QUILT_DOMAIN` name the `.quilt` one, which still keeps the credentials.
fn dataspace_domain_dir() -> Result<PathBuf, Error> {
    dirs::data_local_dir()
        .map(|dir| dir.join(DOMAIN_DIR_NAME))
        .ok_or(Error::Domain)
}

fn get_default_home_dir() -> Result<PathBuf, Error> {
    default_home_dir_under(user_home_dir())
}

fn default_home_dir_under(user_home: Option<PathBuf>) -> Result<PathBuf, Error> {
    user_home
        .map(|user_home| user_home.join(quilt_rs::DEFAULT_HOME_DIR_NAME))
        .ok_or(Error::Home)
}

#[cfg(not(test))]
fn user_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// A temporary folder stands in for the user's home under test: setting the
/// home creates its folder, and a test must not create the real `~/QuiltSync`.
#[cfg(test)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as the real lookup it stands in for"
)]
fn user_home_dir() -> Option<PathBuf> {
    static USER_HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let user_home = USER_HOME
        .get_or_init(|| tempfile::tempdir().expect("a temporary folder for the user home"));
    Some(user_home.path().to_path_buf())
}

/// Make sure the domain has a home before a command runs.
///
/// `home` is the deprecated global `--home`: it warns on `stderr`, then does
/// what `quilt home <dir>` does without `--overwrite`.
async fn initialize_home(
    model: &Model,
    home: Option<PathBuf>,
    stderr: &mut impl std::io::Write,
) -> Result<(), Error> {
    if let Some(dir) = home {
        writeln!(
            stderr,
            "warning: --home is deprecated; use \"quilt home <dir>\""
        )?;
        home::set(model.get_local_domain(), &dir, false, home::Via::HomeFlag).await?;
    } else {
        match model.get_home().await {
            Ok(_) => {}
            Err(Error::Quilt(quilt_rs::Error::Lineage(
                quilt_rs::LineageError::Missing | quilt_rs::LineageError::MissingHome,
            ))) => {
                model.set_home(get_default_home_dir()?).await?;
            }
            Err(err) => return Err(err),
        }
    }

    model.get_home().await?;
    Ok(())
}

fn namespace_from_working_dir(home: &Path, current_dir: &Path) -> Result<Namespace, Error> {
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let current_dir =
        std::fs::canonicalize(current_dir).unwrap_or_else(|_| current_dir.to_path_buf());
    let mut components = current_dir
        .strip_prefix(&home)
        .map_err(|_| Error::NamespaceRequired)?
        .components();

    let prefix = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .ok_or(Error::NamespaceRequired)?;
    let name = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .ok_or(Error::NamespaceRequired)?;

    Namespace::try_from(format!("{prefix}/{name}")).map_err(Error::from)
}

async fn resolve_namespace(model: &Model, namespace: Option<String>) -> Result<Namespace, Error> {
    if let Some(namespace) = namespace {
        return Ok(namespace.try_into()?);
    }

    let home = model.get_home().await?;
    let current_dir = std::env::current_dir()?;
    namespace_from_working_dir(home.as_ref(), &current_dir)
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    command: Commands,

    /// Deprecated: use `quilt home <dir>`. Sets the home before the command,
    /// as `quilt home <dir>` does without `--overwrite`.
    #[arg(long, value_name = "DIR")]
    home: Option<PathBuf>,

    /// Path to local domain
    // Accepts an empty value so `try_parse_with_env` can tell an empty
    // variable (unset) from an empty flag (an error).
    #[arg(
        short,
        long,
        global = true,
        env = DOMAIN_ENV,
        value_parser = clap::builder::OsStringValueParser::new().map(PathBuf::from),
    )]
    domain: Option<PathBuf>,

    /// Show INFO-level logs from quilt on stderr: shorthand for `--log info`.
    // Checked against `--log` in `try_parse_with_env`, as `--json` is.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Logs to show on stderr: a level (`debug`) or tracing directives
    /// (`quilt_rs=trace`). Beats `QUILT_LOG`, which takes the same values.
    #[arg(long, global = true, value_name = "FILTER", value_parser = parse_log_flag)]
    log: Option<String>,

    /// Format of the command's result: `text` (the default) or `json`.
    /// Logs and warnings on stderr are never affected.
    #[arg(
        long,
        global = true,
        env = FORMAT_ENV,
        value_name = "FORMAT",
        value_parser = format_value_parser(),
    )]
    format: Option<String>,

    /// Shorthand for `--format json`.
    // Not clap's `conflicts_with`: that counts a `QUILT_FORMAT` value as given,
    // and misses the pair split across the command. `try_parse_with_env`
    // checks the conflict instead.
    #[arg(long, global = true)]
    json: bool,
}

impl Args {
    /// Parses `argv` as [`Parser::try_parse_from`] does, with `--domain`
    /// falling back to `QUILT_DOMAIN` and `--format` to `QUILT_FORMAT` when
    /// the flag is absent.
    ///
    /// An empty variable counts as unset, so the setting falls back to its
    /// default. An empty flag, `--domain ""` or `--format ""`, stays a usage
    /// error: it usually means an unset shell variable.
    pub fn try_parse_with_env<I, T>(argv: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::try_parse_switched(argv, switch::from_env())
    }

    /// [`Args::try_parse_with_env`] with the switch `on` or off. On, a verb
    /// with a dataspace form that names no dataspace is read as naming the
    /// default one.
    pub fn try_parse_switched<I, T>(argv: I, on: bool) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let argv: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
        let argv = if on {
            switch::with_default_dataspace(argv)
        } else {
            argv
        };
        let mut command = Self::command();
        let mut matches = command.try_get_matches_from_mut(argv)?;
        let domain_source = matches.value_source("domain");
        let format_source = matches.value_source("format");
        let mut parsed =
            Self::from_arg_matches_mut(&mut matches).map_err(|err| err.format(&mut command))?;
        if parsed
            .domain
            .as_ref()
            .is_some_and(|domain| domain.as_os_str().is_empty())
        {
            parsed.domain = None;
            reject_empty_flag(&mut command, domain_source, "--domain <DOMAIN>")?;
        }
        if parsed.format.as_deref() == Some("") {
            parsed.format = None;
            reject_empty_flag(&mut command, format_source, "--format <FORMAT>")?;
        }
        // Only the flag conflicts: a flag beats `QUILT_FORMAT`, and `--json`
        // is a flag.
        if parsed.json && format_source == Some(clap::parser::ValueSource::CommandLine) {
            return Err(command.error(
                clap::error::ErrorKind::ArgumentConflict,
                "the argument '--json' cannot be used with '--format <FORMAT>'",
            ));
        }
        if let Commands::Version { domain_from, .. } = &mut parsed.command
            && parsed.domain.is_some()
        {
            *domain_from = version::DomainFrom::from_source(domain_source);
        }
        if parsed.verbose && parsed.log.is_some() {
            return Err(command.error(
                clap::error::ErrorKind::ArgumentConflict,
                "the argument '--verbose' cannot be used with '--log <FILTER>'",
            ));
        }
        Ok(parsed)
    }

    /// The directives `--log` or `-v` asked for, if either was given.
    pub fn log_directives(&self) -> Option<String> {
        if self.verbose {
            log_filter("info").expect("a level")
        } else {
            self.log.clone()
        }
    }

    /// Whether this run may show the newer-release notice: not while
    /// `quilt upgrade` acts on it, and not from `quilt version`, which writes
    /// nothing and asks nothing of the network.
    pub fn shows_update_notice(&self) -> bool {
        !matches!(self.command, Commands::Upgrade | Commands::Version { .. })
    }

    /// The format of the command's result: `--json` or `--format`, then
    /// `QUILT_FORMAT`, then text.
    pub fn format(&self) -> Format {
        if self.json || self.format.as_deref() == Some("json") {
            Format::Json
        } else {
            Format::Text
        }
    }
}

/// An empty value read from the environment counts as unset. An empty flag
/// stays a usage error: it usually means an unset shell variable.
fn reject_empty_flag(
    command: &mut clap::Command,
    source: Option<clap::parser::ValueSource>,
    flag: &str,
) -> Result<(), clap::Error> {
    if source == Some(clap::parser::ValueSource::EnvVariable) {
        Ok(())
    } else {
        Err(command.error(
            clap::error::ErrorKind::InvalidValue,
            format!("a value is required for '{flag}' but none was supplied"),
        ))
    }
}

/// The package a command acts on.
///
/// An omitted `--namespace` is inferred from the current working directory.
/// `install` keeps its own `namespace`: there, omitting it means "take it from
/// the URI", which is a different question.
#[derive(clap::Args, Debug)]
struct PackageRef {
    /// Namespace of the package. If omitted, infer it from the current
    /// working directory under the configured home.
    #[arg(short, long)]
    namespace: Option<String>,
}

impl PackageRef {
    async fn resolve(self, model: &Model) -> Result<Namespace, Error> {
        resolve_namespace(model, self.namespace).await
    }
}

#[derive(Subcommand, Debug)]
enum LineageAction {
    /// Move this domain's lineage back from per-package refs to data.json,
    /// or finish a rollback that was interrupted
    ///
    /// This quilt, and every quilt or `QuiltSync` that keeps lineage in refs,
    /// moves it there again the next time it opens the domain.
    Rollback,
}

#[derive(Subcommand, Debug)]
enum CatalogAction {
    /// Make this Quilt stack the default
    Set {
        /// Ex. open.quiltdata.com
        host: Host,
    },
    /// Forget the default; quilt3's is not taken again
    Clear,
}

/// Parse `--meta` / `--user-meta` / `-u`. `default` is [`UserMeta::Keep`] on
/// `commit` and [`UserMeta::Clear`] on `create`. Only a JSON object is
/// accepted; any other JSON value is [`Error::CommitMetaInvalid`].
fn parse_meta_flag(text: Option<&str>, default: UserMeta) -> Result<UserMeta, Error> {
    let Some(text) = text else {
        return Ok(default);
    };
    // `UserMeta::parse` always returns `Set`; Keep and Clear are not reachable.
    let UserMeta::Set(value, numbers) = UserMeta::parse(text)? else {
        unreachable!("UserMeta::parse always returns Set");
    };
    if value.is_object() {
        Ok(UserMeta::Set(value, numbers))
    } else {
        Err(Error::CommitMetaInvalid(text.to_string()))
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Browse remote manifest
    Browse {
        #[arg(value_name = "PKG_URI")]
        uri: String,
    },
    /// Show, set or clear the default catalog
    ///
    /// The default is what `login` and `role` use when you leave out `--host`.
    /// It is kept in `settings.json` in the shared folder (`QUILT_AUTH_DIR`),
    /// and is never used for a package: a package keeps the catalog it was
    /// installed from, or none. With nothing stored, it is taken from quilt3's
    /// `config.yml`, which is only read.
    Catalog {
        #[command(subcommand)]
        action: Option<CatalogAction>,
    },
    /// Create a new local package
    ///
    /// With `--dataspace`, declares the package, with no revision, in that
    /// dataspace's `data.toml` under `--access`, `sync` by default, and
    /// commits what its folder holds as its first commit, with no base. The
    /// next sync publishes it, expecting no `latest`. `--source` names an
    /// existing folder to package where it is, inside the dataspace or not;
    /// nothing is copied. Its metadata comes from the folder's
    /// `metadata.toml`.
    Create {
        /// Namespace for the package, e.g. foo/bar
        #[arg(short, long, required_unless_present = "dataspace")]
        namespace: Option<String>,
        /// With --dataspace, the package to create, in a bucket or this
        /// machine's local registry.
        /// Ex. `quilt+s3://bucket#package=foo/bar` or
        /// `quilt+local://#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// Create in this dataspace, the folder holding `data.toml`, rather
        /// than the home. One with no `data.toml` gets one
        #[arg(long, value_name = "DIR", requires = "package")]
        dataspace: Option<PathBuf>,
        /// With --dataspace, the list the package goes in: sync (the
        /// default), write or replace
        #[arg(long, requires = "dataspace")]
        access: Option<Access>,
        /// Optional source directory to populate the package from; with
        /// --dataspace, the package's working folder
        #[arg(short, long)]
        source: Option<PathBuf>,
        /// Commit message for the initial revision
        #[arg(short, long)]
        message: Option<String>,
        /// Package metadata as a JSON object (quilt3's `--meta`). No flag
        /// writes no `user_meta` key. `--user-meta` is the same flag. Only an
        /// object is accepted.
        #[arg(
            short = 'u',
            long = "meta",
            visible_alias = "user-meta",
            value_name = "JSON",
            conflicts_with = "dataspace"
        )]
        user_meta: Option<String>,
    },
    /// Commit new package revision
    ///
    /// With `--dataspace`, commits a `sync` entry's working folder as a local
    /// commit in the Domain, as a sync would, and appends it to
    /// `data.lock.toml` for the next sync to push. Its metadata comes from the
    /// folder's `metadata.toml`, and its workflow is the bucket's default,
    /// checked when it is pushed.
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
        /// Package metadata as a JSON object (quilt3's `--meta`). No flag
        /// keeps the previous revision's metadata. `--user-meta` is the same
        /// flag. Only an object is accepted (`--meta null` is an error). There
        /// is no flag for removing the `user_meta` key (`Clear`).
        #[arg(
            short = 'u',
            long = "meta",
            visible_alias = "user-meta",
            value_name = "JSON",
            conflicts_with = "dataspace"
        )]
        user_meta: Option<String>,
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI, to commit in `--dataspace`; or name it
        /// with `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// Commit in this dataspace, the folder holding `data.toml`, rather
        /// than the home
        #[arg(long, value_name = "DIR")]
        dataspace: Option<PathBuf>,
        /// Workflow ID
        /// Ex. `"my_workflow"`
        /// Omit to use the bucket's default workflow.
        #[arg(short, long, conflicts_with = "dataspace")]
        workflow: Option<String>,
        /// Commit with no workflow (explicit opt-out)
        #[arg(long, conflicts_with_all = ["workflow", "dataspace"])]
        no_workflow: bool,
    },
    /// Make a dataspace package's current revision its `latest`, whatever
    /// `latest` names
    ///
    /// Pushes the package's last local commit, then writes `latest` over
    /// any other writer's, as Promote does. A revision the bucket's workflow
    /// refused is certified all the same, which releases it. Only a `sync`
    /// entry is certified.
    Certify {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI; or name it with `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// The dataspace: the folder holding `data.toml`
        #[arg(long, value_name = "DIR")]
        dataspace: PathBuf,
    },
    /// Delete what the local store holds for nothing, and print what was freed
    ///
    /// Deletes the objects no installed package uses, such as those an
    /// uninstall left, the cache of remote manifests, and files left mid-write
    /// by an interrupted command. Stops without deleting anything if another
    /// quilt process, such as `QuiltSync`, is changing a package.
    ///
    /// With `--dataspace`, clears the dataspace Domain, `bio.quilt.dataspace`,
    /// instead: in each bucket's store, the manifests the bucket holds and the
    /// objects only they use. Commits not pushed and their objects are kept,
    /// the local registry is never touched, and a store whose bucket cannot
    /// be asked, as offline, or that another process is changing, is left as
    /// it is; then this exits 1. Each bucket is asked through the default
    /// catalog, or this machine's AWS credentials where none is set.
    Gc {
        /// Clear the dataspace Domain rather than the home's `.quilt`
        #[arg(long)]
        dataspace: bool,
        /// With --dataspace, print what would be freed, deleting nothing
        #[arg(long, requires = "dataspace")]
        dry_run: bool,
    },
    /// Print the home, or set it with `quilt home <dir>`
    ///
    /// The home is the folder where installed packages keep their files, one
    /// `<prefix>/<name>` folder each. On first use it defaults to `~/QuiltSync`.
    /// `<dir>` may be relative to the current directory, and is created if
    /// missing.
    ///
    /// Changing the home is refused while any package is installed, since
    /// their folders live under the current one.
    ///
    /// With `--dataspace`, prints or chooses the default dataspace, which
    /// replaces the home: `<dir>` is made a dataspace, with an empty
    /// `data.toml`, where it is not one, and nothing moves. The choice is kept
    /// with this application's settings, in `default_dataspace.json`.
    Home {
        /// Folder to use as the home
        #[arg(value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Print or choose the default dataspace rather than the home
        #[arg(long, conflicts_with = "overwrite")]
        dataspace: bool,
        /// Change the home even while packages are installed.
        ///
        /// Their files are not moved: each installed package then points at an
        /// empty folder under the new home, and its files read as deleted. An
        /// app or another `quilt` writing a package at the same time finishes
        /// under the old folder.
        #[arg(long, requires = "dir")]
        overwrite: bool,
    },
    /// Install package locally
    ///
    /// With `--dataspace`, adds the package to that dataspace's `data.toml`
    /// under `--access`, unpinned, then reads it into its working folder and
    /// records the revision in `data.lock.toml`. A revision the URI names is
    /// read and recorded in the lock, not pinned. A package already listed
    /// under another access is refused: edit `data.toml` to change it.
    /// Exits 1 when the package was not read whole.
    #[command(group(clap::ArgGroup::new("keep").args(["keep_all", "keep_folder"])))]
    Install {
        /// Source URI for the package.
        /// Ex. `quilt+s3://bucket#package=foo/bar`, or with `--dataspace`
        /// `quilt+local://#package=foo/bar` for this machine's local registry
        #[arg(value_name = "PKG_URI")]
        uri: String,
        /// Install into this dataspace, the folder holding `data.toml`,
        /// rather than the home. One with no `data.toml` gets one
        #[arg(
            long,
            value_name = "DIR",
            conflicts_with_all = ["namespace", "keep_all", "keep_folder", "no_download"]
        )]
        dataspace: Option<PathBuf>,
        /// With --dataspace, the list the package goes in: read, write,
        /// replace or sync (the default)
        #[arg(long, requires = "dataspace")]
        access: Option<Access>,
        /// With --dataspace, the working folder, relative to the dataspace;
        /// omit to place it by its registry and name
        #[arg(long, value_name = "DIR", requires = "dataspace")]
        folder: Option<PathBuf>,
        /// Namespace for the package, ex. foo/bar.
        #[arg(short, long)]
        namespace: Option<String>,
        /// Logical key relative to the root of the package to be installed locally.
        /// You can provide multiple paths.
        #[arg(short, long)]
        path: Option<Vec<PathBuf>>,
        /// Keep every file in the package, as `quilt keep --all`, and download them
        #[arg(long)]
        keep_all: bool,
        /// Keep every file under this folder, as `quilt keep --folder`, and
        /// download them. Repeat for more folders
        #[arg(long, value_name = "DIR")]
        keep_folder: Vec<String>,
        /// With --keep-all or --keep-folder, save the setting without
        /// downloading anything
        #[arg(long, requires = "keep")]
        no_download: bool,
    },
    /// Where this domain keeps its lineage
    Lineage {
        #[command(subcommand)]
        action: LineageAction,
    },
    /// Show or set which of a package's files this copy keeps
    ///
    /// With no flag, prints the current setting. A setting then downloads the
    /// files it covers that are not here yet; `--no-download` only saves it.
    /// Every `quilt pull` follows the setting, as `QuiltSync`'s pulls do.
    ///
    /// With `--dataspace`, changes the package's entry in that dataspace's
    /// `data.toml`: the paths it covers (`--all`, `--folder`, `--path`), its
    /// pin (`--pin`, `--unpin`), or its list (`--access`). Files a `read` or
    /// `sync` entry newly covers are read at the revision `data.lock.toml`
    /// names before anything can count them deleted; files it no longer
    /// covers stay in the folder. A pin takes effect at the next read. With
    /// no change, prints the entry. Exits 1 when the newly covered files
    /// were not read.
    #[command(group(clap::ArgGroup::new("scope").args(["files", "all", "folder"])))]
    Keep {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI, in `--dataspace`; or name it with
        /// `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// Change what this dataspace, the folder holding `data.toml`, keeps,
        /// rather than the home's copy
        #[arg(long, value_name = "DIR", conflicts_with_all = ["files", "no_download"])]
        dataspace: Option<PathBuf>,
        /// Keep only the files you picked, as `install --path` does
        #[arg(long)]
        files: bool,
        /// Keep every file in the package, including files added later
        #[arg(long)]
        all: bool,
        /// Keep every file under this folder, including files added later.
        /// A path from the package root; it need not exist yet. Repeat for
        /// more folders
        #[arg(long, value_name = "DIR")]
        folder: Vec<String>,
        /// With --dataspace, keep this logical key, or folder of them ending
        /// in `/`. Repeat for more
        #[arg(
            long,
            value_name = "PATH",
            requires = "dataspace",
            conflicts_with = "all"
        )]
        path: Vec<String>,
        /// With --dataspace, pin a `read` entry to this revision: a top hash,
        /// a prefix of one, or a tag after `:`
        #[arg(long, value_name = "REVISION", requires = "dataspace")]
        pin: Option<String>,
        /// With --dataspace, read `latest` again rather than a pinned revision
        #[arg(long, requires = "dataspace", conflicts_with = "pin")]
        unpin: bool,
        /// With --dataspace, move the entry to this list: read, write,
        /// replace or sync
        #[arg(long, requires = "dataspace")]
        access: Option<Access>,
        /// With --dataspace, the list whose entry changes, where the package
        /// is in two
        #[arg(long, value_name = "ACCESS", requires = "dataspace")]
        from: Option<Access>,
        /// Save the setting without downloading anything
        #[arg(long, requires = "scope")]
        no_download: bool,
    },
    /// Print the name of this machine's local registry, or rename it
    ///
    /// The registry is a dataspace Domain's (`bio.quilt.dataspace`). A rename
    /// keeps the old name as an alias, so a `quilt+local://` URI or lock
    /// written with it still names the registry, and moves the autosync work
    /// queued under the old name to the new one.
    LocalRegistry {
        /// The new name: letters, digits, `-` and `.`
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },
    /// Authenticate against a Quilt stack
    Login {
        /// Code from the `https://QUILT_STACK/code` page
        #[arg(short, long)]
        code: Option<String>,
        /// Quilt stack to log in to. Omit to use the default catalog.
        /// Ex. open.quiltdata.com
        #[arg(long)]
        host: Option<Host>,
    },
    /// List installed packages
    ///
    /// Without `--fetch`, reads only what this machine recorded, so each status
    /// is as of the package's last install, pull or push.
    ///
    /// With `--dataspace`, lists the packages that dataspace's `data.toml`
    /// declares, each with its status against `data.lock.toml`, and writes
    /// nothing.
    List {
        /// Check each package's remote now, and count files changed since the
        /// last commit. Reads the remotes; leaves local packages unchanged.
        #[arg(long)]
        fetch: bool,
        /// List this dataspace's packages, the folder holding `data.toml`,
        /// rather than the home's
        #[arg(long, value_name = "DIR")]
        dataspace: Option<PathBuf>,
    },
    /// List the revisions of a package this copy has, newest first.
    ///
    /// Ordered by when this copy obtained each revision, which is all that is
    /// recorded locally — a manifest carries no timestamp of its own, so for a
    /// revision fetched from a remote this is the fetch time, not the commit
    /// time.
    ///
    /// Each revision says why it is kept (current, latest, base, not pushed,
    /// unpublished), or, when `quilt remove-revisions` would remove it, how
    /// much removing it frees. Reading what is kept asks the remote's
    /// registry; if that fails, the log leaves both columns out and warns on
    /// stderr.
    ///
    /// With `--dataspace`, lists the revisions the package's registry has,
    /// newest first by when each was published, and the local commits the
    /// Domain holds that it does not have yet, marked as local. Writes
    /// nothing.
    Log {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI, as the dataspace's `data.toml` names it.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<PackageKey>,
        /// Read the package's history as this dataspace, the folder holding
        /// `data.toml`, has it
        #[arg(long, value_name = "DIR", requires = "package")]
        dataspace: Option<PathBuf>,
    },
    /// Pull
    Pull {
        #[command(flatten)]
        pkg: PackageRef,
    },
    /// Push
    Push {
        #[command(flatten)]
        pkg: PackageRef,
        /// S3 bucket (required for first push of local-only packages)
        #[arg(short, long)]
        bucket: Option<String>,
        /// Quilt stack of the bucket, for a first push (requires --bucket).
        /// Omit to push on this computer's AWS credentials, with no catalog.
        /// Ex. open.quiltdata.com
        #[arg(long, requires = "bucket")]
        host: Option<Host>,
        /// Deprecated: use --host
        #[arg(short, long, requires = "bucket", conflicts_with = "host")]
        origin: Option<Host>,
        /// Workflow ID for the first push (requires --bucket).
        /// Ex. `"my_workflow"`
        /// Omit to use the bucket's default workflow.
        /// Meaningful only on a first push: a subsequent push uploads an
        /// already-created commit whose workflow was chosen at commit time.
        #[arg(short, long)]
        workflow: Option<String>,
        /// First push with no workflow (explicit opt-out; requires --bucket)
        #[arg(long, conflicts_with = "workflow")]
        no_workflow: bool,
    },
    /// Publish a dataspace's `write` and `replace` entries from their working
    /// folders, with no Domain but its local registry
    ///
    /// A `quilt+local://` entry is published to the Domain's local registry,
    /// where a Domain exists, and refused otherwise. Leaves out what the ignore rules leave out of a commit. A package also
    /// under `read` publishes only on its base, and is read first where
    /// `latest` has moved. Records each revision that became `latest` in
    /// `data.lock.toml`. Exits 1 unless `latest` holds every package's folder.
    Publish {
        /// The dataspace: the folder holding `data.toml`
        #[arg(value_name = "DATASPACE")]
        dataspace: PathBuf,
    },
    /// Read a dataspace's `read` entries into their working folders, with no
    /// Domain but its local registry
    ///
    /// A `quilt+local://` entry is read from the Domain's local registry,
    /// where a Domain exists, and left unread otherwise. Records each revision read in `data.lock.toml`. A file changed here is
    /// kept, unless the revision changed it too: then that package is left as
    /// it is. Exits 1 when any package was not read whole.
    Read {
        /// The dataspace: the folder holding `data.toml`
        #[arg(value_name = "DATASPACE")]
        dataspace: PathBuf,
    },
    /// List a package's local commits that the dataspace's lock does not
    /// name, or attach one to the lock again
    ///
    /// The recovery for a deleted `data.lock.toml`. A commit is attached on
    /// the base you name, which its next publish expects `latest` to be, or
    /// with `--unpublished` as a commit of a package never published, whose
    /// next publish expects no `latest`.
    #[command(group(clap::ArgGroup::new("on").args(["base", "unpublished"])))]
    Reattach {
        /// The dataspace: the folder holding `data.toml`
        #[arg(value_name = "DATASPACE")]
        dataspace: PathBuf,
        /// The package as the lock names it.
        /// Ex. `quilt+s3://bucket#package=owner/name`
        #[arg(value_name = "PACKAGE")]
        package: PackageKey,
        /// The commit to attach, one the listing shows; omit to list
        #[arg(value_name = "COMMIT", requires = "on")]
        commit: Option<String>,
        /// The revision the commit was made on, as a whole top hash
        #[arg(long, value_name = "HASH", requires = "commit")]
        base: Option<String>,
        /// The commit was made before the package's first publication, so
        /// it has no base
        #[arg(long, requires = "commit")]
        unpublished: bool,
    },
    /// Remove old revisions of a package, and the stored files only they use
    ///
    /// Removes the revisions this copy holds that it no longer needs, oldest
    /// first, and prints what was freed. A revision is kept while the working
    /// files are at it, while it is the remote's latest or the merge base,
    /// while it is committed but not pushed, and while the registry does not
    /// list it, so this copy is the only place it is.
    ///
    /// With `--dataspace`, removes each `--revision` from the package's
    /// registry instead, as quilt3's `delete_package` with a registry does:
    /// its tags, never its data. Where `latest` names one, it moves to the
    /// newest revision left. No revision is protected, and `data.lock.toml`
    /// is left as it is.
    RemoveRevisions {
        #[command(flatten)]
        pkg: PackageRef,
        /// Remove only the N oldest removable revisions; all of them if omitted
        #[arg(
            short,
            long,
            value_name = "N",
            value_parser = remove_revisions::parse_count,
            conflicts_with = "dataspace"
        )]
        count: Option<std::num::NonZeroUsize>,
        /// The package, by its URI, in `--dataspace`; or name it with
        /// `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// Remove from the registry the package has in this dataspace, the
        /// folder holding `data.toml`
        #[arg(long, value_name = "DIR", requires = "revision")]
        dataspace: Option<PathBuf>,
        /// With --dataspace, a revision to remove: its top hash, or a prefix
        /// of only one. Repeat for more
        #[arg(long, value_name = "HASH", requires = "dataspace")]
        revision: Vec<String>,
    },
    /// Discard a dataspace package's local commits and changes, and read
    /// `latest` whole in their place
    ///
    /// Files the package's revisions hold are replaced or removed; a file no
    /// revision holds is left. The commits stay in the Domain, where
    /// `quilt reattach` lists them.
    Reset {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI; or name it with `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// The dataspace: the folder holding `data.toml`
        #[arg(long, value_name = "DIR")]
        dataspace: PathBuf,
    },
    /// Show or switch your active role on a Quilt stack
    ///
    /// The active role is server-side and global: it decides what every Quilt
    /// client signed in as you can read and write, this CLI and the desktop
    /// app alike. Listing marks the active role with a leading asterisk.
    Role {
        /// Quilt stack to read the role from. Omit to use the default catalog.
        /// Ex. open.quiltdata.com
        #[arg(long)]
        host: Option<Host>,
        /// Switch to this role instead of listing the roles you hold
        #[arg(long, value_name = "ROLE")]
        set: Option<String>,
    },
    /// Status of the package: modified, up-to-date, outdated
    ///
    /// Also says how many files and folders the ignore rules leave out.
    ///
    /// With `--dataspace`, shows each entry of that dataspace's `data.toml`:
    /// its hashes in `data.lock.toml` against the registry's `latest`, its
    /// working folder's changes under the dataspace's ignore policy, its
    /// conflicts in `conflict.json`, a revision autosync kept, and its local
    /// commits not pushed. Writes nothing.
    Status {
        #[command(flatten)]
        pkg: PackageRef,
        /// List each ignored path with the rule that ignored it and where the
        /// rule comes from: a built-in rule, or a rules file and its line.
        #[arg(long)]
        ignored: bool,
        /// Show this dataspace, the folder holding `data.toml`, rather than
        /// one package of the home
        #[arg(long, value_name = "DIR", conflicts_with_all = ["namespace", "ignored"])]
        dataspace: Option<PathBuf>,
    },
    /// Print how much the dataspace Domain, `bio.quilt.dataspace`, holds:
    /// each bucket's store, the local registry, and in all
    ///
    /// Reads no bucket. `quilt gc --dataspace --dry-run` says how much of it
    /// gc would free.
    Storage,
    /// Sync a dataspace's `sync` entries once, as autosync would
    ///
    /// Reads each entry's `latest` where it moved, commits what its working
    /// folder changed as a local commit, and pushes the lock's commits. Until
    /// the dataspace is filled, and after `data.toml` is edited, it only
    /// reads, each entry whole: that is what lets `quilt commit --dataspace`
    /// run again after an edit. Exits 1 unless every entry synced.
    Sync {
        /// Sync this dataspace, the folder holding `data.toml`, rather than
        /// the default one `quilt home --dataspace` chose
        #[arg(long, value_name = "DIR")]
        dataspace: Option<PathBuf>,
        /// The message of each commit the sync makes
        #[arg(short, long, default_value = sync::DEFAULT_MESSAGE)]
        message: String,
    },
    /// Undo the newest commit, restoring the revision before it
    ///
    /// Available while the package's commit chain still reaches back, which in
    /// practice means before its first push. Refuses if any tracked file has
    /// uncommitted changes.
    ///
    /// With `--dataspace`, drops the package's last local commit from
    /// `data.lock.toml` and restores the working folder to the revision
    /// before it, refusing if a file it would touch has changes not
    /// committed. The commit stays in the Domain, for `quilt reattach`. A
    /// pushed commit cannot be undone.
    UndoCommit {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI, in `--dataspace`; or name it with
        /// `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            value_parser = package_uri,
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<String>,
        /// Undo in this dataspace, the folder holding `data.toml`, rather
        /// than the home
        #[arg(long, value_name = "DIR")]
        dataspace: Option<PathBuf>,
    },
    /// Uninstall package from local domain
    ///
    /// Deletes the working files and the installed manifests. The downloaded
    /// objects stay for `quilt gc` unless `--prune` is given.
    ///
    /// With `--dataspace`, removes the package from that dataspace's
    /// `data.toml` and `data.lock.toml`, and deletes its working folder if a
    /// read filled it inside the dataspace. Local commits not pushed stay in
    /// the Domain, for `quilt reattach`. `--prune` then clears what the
    /// Domain holds of the package that its bucket holds too.
    Uninstall {
        #[command(flatten)]
        pkg: PackageRef,
        /// The package, by its URI, to uninstall from `--dataspace`; or name
        /// it with `--namespace`.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(
            value_name = "PACKAGE",
            requires = "dataspace",
            conflicts_with = "namespace"
        )]
        package: Option<PackageKey>,
        /// Uninstall from this dataspace, the folder holding `data.toml`,
        /// rather than the home
        #[arg(long, value_name = "DIR")]
        dataspace: Option<PathBuf>,
        /// Also delete the package's downloaded objects that no other
        /// installed package uses, and print what was freed
        #[arg(long)]
        prune: bool,
    },
    /// Install the latest quilt-cli with cargo
    ///
    /// Runs `cargo binstall quilt-cli` when cargo-binstall is installed. When
    /// no prebuilt binary can be installed, it asks before building from
    /// source with `cargo install --locked quilt-cli`, and without a terminal
    /// it does not build. On Windows it prints the command to run instead.
    Upgrade,
    /// Print the version
    Version {
        /// Also print the library versions, the platform, and the folders
        /// quilt uses, for bug reports
        #[arg(long)]
        details: bool,
        // Filled in by `try_parse_with_env`, which sees where `--domain` came from.
        #[arg(skip)]
        domain_from: version::DomainFrom,
    },
}

/// The line a writer prints when another quilt process, such as `QuiltSync`,
/// is writing the same package and this one waits for it.
pub const WAITING_NOTICE: &str = "waiting for another quilt process…";

/// Hands [`WAITING_NOTICE`] to `print` whenever a writer finds its package's
/// lock held and is about to wait. `main` prints it to stderr, so `--json`
/// output on stdout stays clean.
pub fn notice_lock_waits(print: impl Fn(&str) + Send + Sync + 'static) {
    quilt_rs::on_package_lock_wait(move || print(WAITING_NOTICE));
}

/// The Quilt stack `quilt push` names, from `--host` or the deprecated
/// `--origin`/`-o`, which warns on `stderr`. Clap rejects passing both.
fn push_host(
    host: Option<Host>,
    origin: Option<Host>,
    stderr: &mut impl std::io::Write,
) -> Result<Option<Host>, Error> {
    if origin.is_some() {
        writeln!(stderr, "warning: --origin is deprecated; use --host")?;
    }
    Ok(host.or(origin))
}

impl Commands {
    /// Whether a new domain gets the default home before this command runs.
    ///
    /// `login`, `browse` and `role` only talk to a remote and never read the
    /// home, so they don't set it: a default folder that can't be created must
    /// not block them. `quilt home <dir>` sets its own, so defaulting first
    /// would only write a home it is about to replace.
    fn needs_default_home(&self) -> bool {
        !matches!(
            self,
            Commands::Login { .. }
                | Commands::Browse { .. }
                | Commands::Catalog { .. }
                | Commands::Role { .. }
                | Commands::Home { dir: Some(_), .. }
                | Commands::Upgrade
                | Commands::Version { .. }
        )
    }

    /// The folder a dataspace form names as its dataspace, for the default
    /// dataspace to replace [`switch::DEFAULT_MARK`].
    fn dataspace_mut(&mut self) -> Option<&mut PathBuf> {
        match self {
            Commands::Install {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Uninstall {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Status {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::List {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Log {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Commit {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::UndoCommit {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::RemoveRevisions {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Keep {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Create {
                dataspace: Some(dataspace),
                ..
            }
            | Commands::Read { dataspace }
            | Commands::Publish { dataspace }
            | Commands::Reattach { dataspace, .. }
            | Commands::Reset { dataspace, .. }
            | Commands::Certify { dataspace, .. } => Some(dataspace),
            _ => None,
        }
    }
}

pub async fn init(args: Args) -> Result<Std, Error> {
    // NOTE: every command should have some domain,
    //       because domain stores credentials
    //       It's optional for user, but we use one anyway.
    //       If it is None, we use:
    //         * home directory ~/.local/share/com.quiltdata.quilt-sync`
    //         * or temporary directory
    if let Commands::Version {
        details,
        domain_from,
    } = args.command
    {
        let domain = match args.domain {
            Some(domain) => Some(domain),
            None => default_domain_dir(),
        };
        return Ok(version::command(version::Input {
            details,
            domain,
            domain_from,
        })
        .await);
    }
    let on = switch::settled();
    let places = switch::Places::new(on, args.domain.clone())?;
    Box::pin(init_in(args, places, on)).await
}

/// [`init`] for every command but `version`, with the application's folder
/// and the Domain where the switch puts them. With the switch `on`, a
/// dataspace named [`switch::DEFAULT_MARK`] is the default dataspace.
#[allow(
    clippy::too_many_lines,
    reason = "cohesive top-level CLI command dispatch"
)]
async fn init_in(mut args: Args, places: switch::Places, on: bool) -> Result<Std, Error> {
    if let Some(dataspace) = args.command.dataspace_mut()
        && on
        && dataspace.as_os_str() == switch::DEFAULT_MARK
    {
        *dataspace = places.default_dataspace().await?;
    }
    let root_dir = places.app.clone();
    // The dataspace commands use no lineage, so they run before anything
    // touches it; they reach the Domain's local registry where one exists.
    if let Commands::Sync { dataspace, message } = args.command {
        let dataspace = match dataspace {
            Some(dataspace) => dataspace,
            None => places.default_dataspace().await?,
        };
        let input = sync::Input { dataspace, message };
        log::debug!("Syncing a dataspace {input:?}");
        let syncing = sync::command(root_dir, places.domain()?, input);
        return Ok(Box::pin(syncing).await);
    }
    if let Commands::Read { dataspace } = args.command {
        let input = read::Input { dataspace };
        return Ok(read::command(root_dir, places.existing_domain(), input).await);
    }
    if let Commands::Publish { dataspace } = args.command {
        let input = publish::Input { dataspace };
        return Ok(publish::command(root_dir, places.existing_domain(), input).await);
    }
    if let Commands::Install {
        dataspace: Some(dataspace),
        uri,
        access,
        folder,
        path,
        ..
    } = args.command
    {
        let input = install::dataspace::Input {
            dataspace,
            uri,
            access: access.unwrap_or(Access::Sync),
            paths: path.unwrap_or_default(),
            folder,
        };
        log::debug!("Installing into a dataspace {input:?}");
        let installing = install::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(installing).await);
    }
    if let Commands::Uninstall {
        dataspace: Some(dataspace),
        package,
        pkg,
        prune,
    } = args.command
    {
        let input = uninstall::dataspace::Input {
            dataspace,
            package: dataspace::Package::named(package, pkg.namespace)?,
            prune,
        };
        log::debug!("Uninstalling from a dataspace {input:?}");
        let uninstalling = uninstall::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(uninstalling).await);
    }
    if let Commands::Reattach {
        dataspace,
        package,
        commit,
        base,
        unpublished: _,
    } = args.command
    {
        // clap gives a commit `--base` or `--unpublished`, never both.
        let input = reattach::Input {
            dataspace,
            package,
            attach: commit.map(|commit| (commit, base)),
        };
        let domain = places.domain()?;
        return Ok(reattach::command(root_dir, domain, input).await);
    }
    // The dataspace queries write nothing, and reach the Domain's stores
    // where one exists.
    if let Commands::Status {
        dataspace: Some(dataspace),
        ..
    } = args.command
    {
        let input = status::dataspace::Input { dataspace };
        log::debug!("Status of a dataspace {input:?}");
        let reading = status::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(reading).await);
    }
    if let Commands::List {
        dataspace: Some(dataspace),
        fetch,
    } = args.command
    {
        let input = list::dataspace::Input { dataspace, fetch };
        log::debug!("Listing a dataspace {input:?}");
        let listing = list::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(listing).await);
    }
    if let Commands::Log {
        dataspace: Some(dataspace),
        package: Some(package),
        ..
    } = args.command
    {
        let input = history::dataspace::Input { dataspace, package };
        log::debug!("Log of a dataspace's package {input:?}");
        let reading = history::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(reading).await);
    }
    if let Commands::Commit {
        dataspace: Some(dataspace),
        package,
        pkg,
        message,
        ..
    } = args.command
    {
        let input = commit::dataspace::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
            message,
        };
        log::debug!("Committing in a dataspace {input:?}");
        let domain = places.domain()?;
        let committing = commit::dataspace::command(root_dir, domain, input);
        return Ok(Box::pin(committing).await);
    }
    if let Commands::UndoCommit {
        dataspace: Some(dataspace),
        package,
        pkg,
    } = args.command
    {
        let input = undo_commit::dataspace::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
        };
        log::debug!("Undoing a commit in a dataspace {input:?}");
        let domain = places.domain()?;
        let undoing = undo_commit::dataspace::command(root_dir, domain, input);
        return Ok(Box::pin(undoing).await);
    }
    if let Commands::Reset {
        dataspace,
        package,
        pkg,
    } = args.command
    {
        let input = reset::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
        };
        log::debug!("Resetting in a dataspace {input:?}");
        let domain = places.domain()?;
        return Ok(Box::pin(reset::command(root_dir, domain, input)).await);
    }
    if let Commands::Certify {
        dataspace,
        package,
        pkg,
    } = args.command
    {
        let input = certify::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
        };
        log::debug!("Certifying in a dataspace {input:?}");
        let domain = places.domain()?;
        return Ok(Box::pin(certify::command(root_dir, domain, input)).await);
    }
    if let Commands::RemoveRevisions {
        dataspace: Some(dataspace),
        package,
        pkg,
        revision,
        ..
    } = args.command
    {
        let input = remove_revisions::dataspace::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
            revisions: revision,
        };
        log::debug!("Removing revisions from a registry {input:?}");
        let removing =
            remove_revisions::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(removing).await);
    }
    if let Commands::Keep {
        dataspace: Some(dataspace),
        package,
        pkg,
        all,
        folder,
        path,
        pin,
        unpin,
        access,
        from,
        ..
    } = args.command
    {
        let input = keep::dataspace::Input {
            dataspace,
            package: dataspace::Package::uri(package, pkg.namespace)?,
            from,
            paths: keep::dataspace::scope_from(all, folder, path),
            revision: keep::dataspace::pin_from(pin, unpin),
            access,
        };
        log::debug!("Keeping in a dataspace {input:?}");
        let keeping = keep::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(keeping).await);
    }
    if let Commands::Create {
        dataspace: Some(dataspace),
        package: Some(uri),
        access,
        source,
        message,
        ..
    } = args.command
    {
        let input = create::dataspace::Input {
            dataspace,
            uri,
            access: access.unwrap_or(Access::Sync),
            folder: source,
            message,
        };
        log::debug!("Creating in a dataspace {input:?}");
        let domain = places.domain()?;
        let creating = create::dataspace::command(root_dir, domain, input);
        return Ok(Box::pin(creating).await);
    }
    if let Commands::Gc {
        dataspace: true,
        dry_run,
    } = args.command
    {
        let input = gc::dataspace::Input { dry_run };
        log::debug!("Clearing the dataspace Domain {input:?}");
        let clearing = gc::dataspace::command(root_dir, places.existing_domain(), input);
        return Ok(Box::pin(clearing).await);
    }
    if let Commands::Storage = args.command {
        return Ok(Box::pin(storage::command(places.existing_domain())).await);
    }
    if let Commands::LocalRegistry { name } = args.command {
        let input = local_registry::Input { name };
        let domain = places.domain()?;
        return Ok(Box::pin(local_registry::command(domain, input)).await);
    }
    if let Commands::Home {
        dir,
        dataspace: true,
        ..
    } = args.command
    {
        let input = home::dataspace::Input { dir };
        return Ok(Box::pin(home::dataspace::command(root_dir, input)).await);
    }
    // Signing in and roles keep to the application's folder: they use its
    // remote, not the domain's.
    let remote = app_remote(&root_dir);
    let m = Model::from(root_dir);
    // Every other command refuses a rollback that was interrupted, so this
    // one runs before anything reads the lineage.
    if let Commands::Lineage {
        action: LineageAction::Rollback,
    } = args.command
    {
        return Ok(lineage::command(m).await);
    }
    if let Err(err) = m.get_local_domain().sweep_leftovers().await {
        log::warn!("Could not sweep what killed lineage writes left: {err}");
    }

    // Preserve an existing home, honor the deprecated --home, and set the
    // default for a new domain on first use.
    if matches!(args.command, Commands::Home { dir: Some(_), .. }) && args.home.is_some() {
        return Err(Error::HomeTwice);
    }
    if args.home.is_some() || args.command.needs_default_home() {
        initialize_home(&m, args.home, &mut std::io::stderr()).await?;
    }

    match args.command {
        Commands::Browse { uri } => {
            let args = browse::Input { uri };

            log::debug!("Browsing {args:?}");
            Ok(browse::command(m, args).await)
        }
        Commands::Catalog { action } => {
            let action = match action {
                None => catalog::Action::Show,
                Some(CatalogAction::Set { host }) => catalog::Action::Set(host),
                Some(CatalogAction::Clear) => catalog::Action::Clear,
            };
            let args = catalog::Input { action };

            log::debug!("Catalog {args:?}");
            Ok(catalog::command(m, args).await)
        }
        Commands::Create {
            namespace,
            source,
            message,
            user_meta,
            ..
        } => {
            let args = create::Input {
                namespace: namespace.ok_or(Error::NamespaceRequired)?.try_into()?,
                source,
                message,
                user_meta: parse_meta_flag(user_meta.as_deref(), UserMeta::Clear)?,
            };

            log::debug!("Creating {args:?}");
            Ok(create::command(m, args).await)
        }
        Commands::Commit {
            pkg,
            message,
            user_meta,
            workflow,
            no_workflow,
            ..
        } => {
            let namespace = pkg.resolve(&m).await?;
            let user_meta = parse_meta_flag(user_meta.as_deref(), UserMeta::Keep)?;
            let workflow = commit_workflow_intent(workflow.as_deref(), no_workflow)?;
            let args = commit::Input {
                message,
                namespace,
                user_meta,
                workflow,
                host_config: None,
            };

            log::debug!("Committing {args:?}");
            Ok(commit::command(m, args).await)
        }
        Commands::Gc { .. } => {
            log::debug!("Collecting garbage");
            Ok(gc::command(m).await)
        }
        Commands::Home { dir, overwrite, .. } => {
            let args = home::Input { dir, overwrite };

            log::debug!("Home {args:?}");
            Ok(home::command(m, args).await)
        }
        Commands::Install {
            namespace,
            path,
            uri,
            keep_all,
            keep_folder,
            no_download,
            ..
        } => {
            let args = install::Input {
                namespace: parse_optional_namespace(namespace)?,
                paths: path,
                uri,
                keep: keep::scope_from(false, keep_all, keep_folder)?,
                download: !no_download,
            };

            log::debug!("Installing {args:?}");
            Ok(install::command(m, args).await)
        }
        Commands::Keep {
            pkg,
            files,
            all,
            folder,
            no_download,
            ..
        } => {
            let namespace = pkg.resolve(&m).await?;
            let args = keep::Input {
                namespace,
                scope: keep::scope_from(files, all, folder)?,
                download: !no_download,
            };

            log::debug!("Keep {args:?}");
            Ok(keep::command(m, args).await)
        }
        Commands::Login { code, host } => {
            let host = catalog::host_or_default(host).await?;
            if let Some(code) = code {
                let args = login::Input { code, host };

                log::debug!("Logging in {args:?}");
                Ok(login::command(&remote, args).await)
            } else {
                // TODO: Check the lineage, if there are some `package.remote.catalog`
                Ok(Std::Err(Error::LoginRequired(host)))
            }
        }
        Commands::List { fetch, .. } => {
            let args = list::Input { fetch };

            log::info!("Listing installed packages {args:?}");
            Ok(list::command(m, args).await)
        }
        Commands::Log { pkg, .. } => {
            let namespace = pkg.resolve(&m).await?;
            let args = history::Input { namespace };

            log::debug!("Logging {args:?}");
            Ok(history::command(m, args).await)
        }
        Commands::Pull { pkg } => {
            let namespace = pkg.resolve(&m).await?;
            let args = pull::Input {
                namespace,
                host_config: None,
            };

            log::debug!("Pull {args:?}");
            Ok(pull::command(m, args).await)
        }
        Commands::Push {
            pkg,
            bucket,
            host,
            origin,
            workflow,
            no_workflow,
        } => {
            let origin = push_host(host, origin, &mut std::io::stderr())?;
            let namespace = pkg.resolve(&m).await?;
            // The workflow flags only take effect on a first push, where
            // set_remote→recommit resolves them. On a subsequent push the
            // workflow was already decided at commit time, so reject the flags
            // here rather than silently ignore them.
            if bucket.is_none() && (workflow.is_some() || no_workflow) {
                return Err(Error::WorkflowRequiresBucket);
            }
            let workflow = commit_workflow_intent(workflow.as_deref(), no_workflow)?;
            let args = push::Input {
                namespace,
                host_config: None,
                bucket,
                origin,
                workflow,
            };

            log::debug!("Pushing {args:?}");
            Ok(push::command(m, args).await)
        }
        Commands::RemoveRevisions { pkg, count, .. } => {
            let namespace = pkg.resolve(&m).await?;
            let args = remove_revisions::Input { namespace, count };

            log::debug!("Removing old revisions {args:?}");
            Ok(remove_revisions::command(m, args).await)
        }
        Commands::Role { host, set } => {
            let host = catalog::host_or_default(host).await?;
            let args = role::Input { host, set };

            log::debug!("Role {args:?}");
            Ok(role::command(&remote, args).await)
        }
        Commands::Status { pkg, ignored, .. } => {
            let namespace = pkg.resolve(&m).await?;
            let args = status::Input {
                namespace,
                host_config: None,
                list_ignored: ignored,
            };

            log::debug!("Status {args:?}");
            Ok(status::command(m, args).await)
        }
        Commands::UndoCommit { pkg, .. } => {
            let namespace = pkg.resolve(&m).await?;
            let args = undo_commit::Input { namespace };

            log::debug!("Undoing commit {args:?}");
            Ok(undo_commit::command(m, args).await)
        }
        Commands::Uninstall { pkg, prune, .. } => {
            let namespace = pkg.resolve(&m).await?;
            let args = uninstall::Input { namespace, prune };

            log::debug!("Uninstalling {args:?}");
            Ok(uninstall::command(m, args).await)
        }
        Commands::Upgrade => Ok(upgrade::command()),
        Commands::Version { .. }
        | Commands::Read { .. }
        | Commands::Publish { .. }
        | Commands::Reattach { .. }
        | Commands::Reset { .. }
        | Commands::Certify { .. }
        | Commands::Storage
        | Commands::Sync { .. }
        | Commands::LocalRegistry { .. } => {
            unreachable!("handled before the domain is opened")
        }
        Commands::Lineage { .. } => unreachable!("handled before the lineage is read"),
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Domain directory is required. We store files and credentials there")]
    Domain,

    #[error("Could not determine the default home directory. Run \"quilt home <dir>\" to set one")]
    Home,

    #[error("{reason}. Moving the home isn't supported yet. {retry}")]
    HomeInUse { reason: String, retry: home::Retry },

    #[error("--home and \"quilt home <dir>\" both set the home; pass only \"quilt home <dir>\"")]
    HomeTwice,

    #[error("Cannot free space: {0} is busy in another quilt process. Try again once it finishes")]
    PackageBusy(Namespace),

    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),

    #[error(
        r"
Please visit https://{0}/code to get your code.
Then run:
> quilt login --host {0} --code YOUR_CODE"
    )]
    LoginRequired(Host),

    #[error("No catalog given and no default set. Pass --host, or run \"quilt catalog set HOST\"")]
    NoCatalog,

    #[error("Could not determine the folder for shared settings. Set QUILT_AUTH_DIR")]
    SettingsDir,

    #[error("Package {0} not found")]
    NamespaceNotFound(Namespace),

    #[error(transparent)]
    Dataspace(#[from] ReadAllError),

    #[error(transparent)]
    Publish(#[from] PublishAllError),

    #[error(transparent)]
    Reattach(#[from] ReattachError),

    #[error(transparent)]
    Install(#[from] InstallError),

    #[error(transparent)]
    Uninstall(#[from] UninstallError),

    #[error(transparent)]
    Status(#[from] StatusError),

    #[error(transparent)]
    History(#[from] HistoryError),

    #[error(transparent)]
    Lock(#[from] LockError),

    #[error(transparent)]
    Contract(#[from] ContractError),

    #[error(transparent)]
    Commit(#[from] CommitError),

    #[error(transparent)]
    Undo(#[from] UndoError),

    #[error(transparent)]
    Reset(#[from] ResetError),

    #[error(transparent)]
    Certify(#[from] CertifyError),

    #[error(transparent)]
    Remove(#[from] RemoveError),

    #[error(transparent)]
    Keep(#[from] KeepError),

    #[error(transparent)]
    Create(#[from] CreateError),

    #[error(transparent)]
    DataspaceHome(#[from] HomeError),

    #[error(transparent)]
    Rename(#[from] RenameError),

    #[error(transparent)]
    Sync(#[from] SyncError),

    #[error("Can't open the Domain's queue: {0}")]
    Queue(#[from] QueueError),

    #[error("There is no dataspace Domain here yet, so no local registry to name")]
    NoDomain,

    #[error("No default dataspace is chosen. Run \"quilt home --dataspace <dir>\" to choose one")]
    NoDefaultDataspace,

    #[error("Name the package in the dataspace: its URI, or --namespace")]
    PackageRequired,

    #[error("{0} is in more than one registry in data.toml; name the one you mean by its URI")]
    AmbiguousPackage(Namespace),

    #[error("--no-download downloads nothing, so it cannot install --path or a &path= in the URI")]
    NoDownloadWithPath,

    #[error(
        "Could not infer a namespace from the current directory. Pass --namespace or run inside <home>/<prefix>/<name>"
    )]
    NamespaceRequired,

    #[error("Invalid --meta: a JSON object is required, got {0}")]
    CommitMetaInvalid(String),

    #[error("workflow id cannot be empty; omit --workflow to use the bucket's default workflow")]
    WorkflowEmpty,

    #[error(
        "--workflow/--no-workflow apply only to a first push; pass --bucket to set the remote, or omit them (the workflow was chosen at commit time)"
    )]
    WorkflowRequiresBucket,

    #[error(
        "cargo is not installed. Download the latest quilt-cli from https://github.com/quiltdata/quilt-rs/releases"
    )]
    CargoMissing,

    #[error("\"{0}\" failed; its output is above")]
    UpgradeFailed(upgrade::Installer),

    #[error("No prebuilt quilt-cli was installed. Run \"{0}\" to build it from source")]
    NotBuilt(upgrade::Installer),

    #[error("quilt cannot replace itself on Windows while it runs. Run \"{0}\" instead")]
    UpgradeUnsupported(upgrade::Installer),

    #[error("QUILT_LOG={0}")]
    LogEnv(quilt_rs::logging::InvalidLogFilter),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(test)]
    #[error("Test failed: {0}")]
    Test(String),

    #[error("Failed to write or read: {0}")]
    Io(#[from] std::io::Error),
}

impl From<quilt_rs::Error> for Error {
    fn from(err: quilt_rs::Error) -> Error {
        Error::Quilt(err)
    }
}

#[cfg(test)]
impl From<quilt_rs::LogicalKeyError> for Error {
    fn from(err: quilt_rs::LogicalKeyError) -> Error {
        Error::Quilt(err.into())
    }
}

impl From<quilt_uri::UriError> for Error {
    fn from(err: quilt_uri::UriError) -> Error {
        Error::Quilt(quilt_rs::Error::Uri(err))
    }
}

impl Error {
    /// A stable machine-readable discriminant for `--json` consumers, so a
    /// caller can branch without matching English prose.
    ///
    /// Reaches one level into `quilt_rs::Error`, under any failed upload, for
    /// the variants a caller can act on. Everything else is `quilt_error`: a new library variant lands in
    /// the catch-all rather than breaking the build, and earns its own kind only
    /// when something needs to branch on it.
    #[allow(clippy::too_many_lines, reason = "one table of every error's kind")]
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Domain => "domain",
            Error::Home => "home",
            Error::HomeInUse { .. } => "home_in_use",
            Error::HomeTwice => "home_twice",
            Error::PackageBusy(_) => "package_busy",
            Error::Quilt(err) => match err.cause() {
                quilt_rs::Error::Uri(_) => "invalid_uri",
                quilt_rs::Error::Auth(..) => "auth",
                quilt_rs::Error::Login(_) => "login",
                quilt_rs::Error::Lineage(quilt_rs::LineageError::FolderOutsidePackage(_)) => {
                    "folder_outside_package"
                }
                quilt_rs::Error::Lineage(_) => "lineage",
                quilt_rs::Error::WorkflowValidation(_) => "workflow_validation",
                _ => "quilt_error",
            },
            Error::LoginRequired(_) => "login_required",
            Error::NoCatalog => "no_catalog",
            Error::SettingsDir => "settings_dir",
            Error::NamespaceNotFound(_) => "namespace_not_found",
            Error::Dataspace(err) => match err {
                ReadAllError::Contract(_) => "contract",
                ReadAllError::Lock(_) => "lock",
                ReadAllError::Mismatch(_) => "lock_mismatch",
                _ => "dataspace",
            },
            Error::Publish(err) => match err {
                PublishAllError::Contract(_) => "contract",
                PublishAllError::Lock(_) => "lock",
                PublishAllError::Mismatch(_) => "lock_mismatch",
                _ => "dataspace",
            },
            Error::Reattach(err) => match err {
                ReattachError::Contract(_) => "contract",
                ReattachError::Lock(_) => "lock",
                ReattachError::Mismatch(_) => "lock_mismatch",
                ReattachError::Undeclared => "not_declared",
                ReattachError::Attached => "commits_attached",
                ReattachError::NotUnattached(_) => "not_unattached",
                ReattachError::Base(_) => "invalid_base",
                _ => "dataspace",
            },
            Error::Install(err) => match err {
                InstallError::Contract(_) => "contract",
                InstallError::Lock(_) => "lock",
                InstallError::Mismatch(_) => "lock_mismatch",
                InstallError::Uri { .. } => "invalid_uri",
                InstallError::Access { .. } => "access_declared",
                InstallError::Folder { .. } => "folder_declared",
                InstallError::Catalog { .. } => "catalog_declared",
                InstallError::Pinned { .. } => "pinned",
                InstallError::RevisionNotRead(_) => "revision_not_read",
                InstallError::LocalRegistry => "local_registry",
                _ => "dataspace",
            },
            Error::Uninstall(err) => match err {
                UninstallError::Contract(_) => "contract",
                UninstallError::Lock(_) => "lock",
                UninstallError::Mismatch(_) => "lock_mismatch",
                UninstallError::NotInstalled(_) => "not_installed",
                _ => "dataspace",
            },
            Error::Lock(_) => "lock",
            Error::Contract(_) => "contract",
            Error::Commit(_)
            | Error::Undo(_)
            | Error::Reset(_)
            | Error::Certify(_)
            | Error::Remove(_) => self.revision_kind(),
            Error::Keep(_)
            | Error::Create(_)
            | Error::DataspaceHome(_)
            | Error::Rename(_)
            | Error::Queue(_)
            | Error::NoDomain
            | Error::NoDefaultDataspace => self.keep_kind(),
            Error::PackageRequired => "package_required",
            Error::AmbiguousPackage(_) => "ambiguous_package",
            Error::Status(err) => match err {
                StatusError::Contract(_) => "contract",
                StatusError::Lock(_) => "lock",
                _ => "dataspace",
            },
            Error::History(err) => match err {
                HistoryError::Contract(_) => "contract",
                HistoryError::Lock(_) => "lock",
                _ => "dataspace",
            },
            Error::Sync(err) => match err {
                SyncError::Contract(_) => "contract",
                SyncError::Lock(_) => "lock",
                SyncError::Mismatch(_) => "lock_mismatch",
                _ => "dataspace",
            },
            Error::NoDownloadWithPath => "no_download_with_path",
            Error::NamespaceRequired => "namespace_required",
            Error::CommitMetaInvalid(_) => "commit_meta_invalid",
            Error::WorkflowEmpty => "workflow_empty",
            Error::WorkflowRequiresBucket => "workflow_requires_bucket",
            Error::CargoMissing => "cargo_missing",
            Error::UpgradeFailed(_) | Error::NotBuilt(_) => "upgrade_failed",
            Error::UpgradeUnsupported(_) => "upgrade_unsupported",
            Error::LogEnv(_) => "invalid_log_filter",
            Error::Json(_) => "invalid_json",
            #[cfg(test)]
            Error::Test(_) => "internal",
            Error::Io(_) => "io",
        }
    }

    /// [`Error::kind`] of a dataspace command that moves a package's
    /// revisions: commit, undo, reset, certify and remove.
    fn revision_kind(&self) -> &'static str {
        match self {
            Error::Commit(err) => match err {
                CommitError::Contract(_) => "contract",
                CommitError::Lock(_) => "lock",
                CommitError::Mismatch(_) => "lock_mismatch",
                CommitError::NotDeclared(_) => "not_declared",
                CommitError::Access(_) => "access_declared",
                CommitError::Edited => "contract_edited",
                CommitError::NoFolder(_) => "no_folder",
                CommitError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            Error::Undo(err) => match err {
                UndoError::Contract(_) => "contract",
                UndoError::Lock(_) => "lock",
                UndoError::Mismatch(_) => "lock_mismatch",
                UndoError::NotDeclared(_) => "not_declared",
                UndoError::NothingToUndo(_) => "nothing_to_undo",
                UndoError::Uncommitted(_) => "uncommitted_changes",
                UndoError::Unfetched(_) => "unfetched",
                UndoError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            Error::Reset(err) => match err {
                ResetError::Contract(_) => "contract",
                ResetError::Lock(_) => "lock",
                ResetError::Mismatch(_) => "lock_mismatch",
                ResetError::NotDeclared(_) => "not_declared",
                ResetError::Access(_) => "access_declared",
                ResetError::Pinned(_) => "pinned",
                ResetError::NoLatest => "no_latest",
                ResetError::Unfetched(_) => "unfetched",
                ResetError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            Error::Certify(err) => match err {
                CertifyError::Contract(_) => "contract",
                CertifyError::Lock(_) => "lock",
                CertifyError::Mismatch(_) => "lock_mismatch",
                CertifyError::NotDeclared(_) => "not_declared",
                CertifyError::Access(_) => "access_declared",
                CertifyError::NothingToCertify(_) => "nothing_to_certify",
                CertifyError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            Error::Remove(err) => match err {
                RemoveError::Contract(_) => "contract",
                RemoveError::Lock(_) => "lock",
                RemoveError::Mismatch(_) => "lock_mismatch",
                RemoveError::NotDeclared(_) => "not_declared",
                RemoveError::NoneNamed => "revision_required",
                RemoveError::NotFound(_) => "revision_not_found",
                RemoveError::Ambiguous(_) => "ambiguous_revision",
                RemoveError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            _ => "dataspace",
        }
    }

    /// [`Error::kind`] of a dataspace command that changes what is kept:
    /// keep, create, the default dataspace, gc and the local registry's name.
    fn keep_kind(&self) -> &'static str {
        match self {
            Error::Keep(err) => match err {
                KeepError::Contract(_) => "contract",
                KeepError::Lock(_) => "lock",
                KeepError::Mismatch(_) => "lock_mismatch",
                KeepError::NotDeclared(_) => "not_declared",
                KeepError::NotUnder { .. } => "not_under_access",
                KeepError::Ambiguous { .. } => "ambiguous_access",
                KeepError::Path { .. } | KeepError::Paths => "invalid_path",
                KeepError::Revision(_) => "invalid_revision",
                _ => "dataspace",
            },
            Error::Create(err) => match err {
                CreateError::Contract(_) => "contract",
                CreateError::Lock(_) => "lock",
                CreateError::Mismatch(_) => "lock_mismatch",
                CreateError::Uri { .. } => "invalid_uri",
                CreateError::Named { .. } => "invalid_name",
                CreateError::Read => "access_not_created",
                CreateError::Declared(_) => "already_declared",
                CreateError::Exists(_) => "already_exists",
                CreateError::LocalRegistry => "local_registry",
                CreateError::HoldsDomain(_) => "folder_holds_domain",
                CreateError::Busy(_) => "package_busy",
                _ => "dataspace",
            },
            Error::DataspaceHome(HomeError::Contract(_)) => "contract",
            Error::Rename(err) => match err {
                RenameError::Name(_) => "invalid_name",
                RenameError::Queue(_) => "queue",
                _ => "dataspace",
            },
            Error::Queue(_) => "queue",
            Error::NoDomain => "no_domain",
            Error::NoDefaultDataspace => "no_default_dataspace",
            _ => "dataspace",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quilt_uri::fixtures;
    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    /// A writer whose package another process holds says it is waiting,
    /// once, then runs when the other lets go.
    #[test(tokio::test)]
    async fn a_writer_waiting_on_a_held_package_says_so() -> Result<(), Error> {
        let printed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        notice_lock_waits({
            let printed = std::sync::Arc::clone(&printed);
            move |line| printed.lock().expect("printed").push(line.to_string())
        });
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let namespace: Namespace = ("test", "held").into();
        let created = m
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: None,
                user_meta: UserMeta::Clear,
            })
            .await?;
        let held = created.installed_package.lock().await?;

        let mut uninstall = std::pin::pin!(m.uninstall(uninstall::Input {
            namespace,
            prune: false,
        }));
        let early =
            tokio::time::timeout(std::time::Duration::from_millis(300), &mut uninstall).await;
        assert!(early.is_err(), "the uninstall waits for the holder");
        assert_eq!(*printed.lock().expect("printed"), vec![WAITING_NOTICE]);

        drop(held);
        tokio::time::timeout(std::time::Duration::from_secs(10), uninstall)
            .await
            .expect("the uninstall runs once the holder lets go")?;
        assert_eq!(printed.lock().expect("printed").len(), 1, "said once");
        Ok(())
    }

    /// The hint is a command the reader is meant to run, and since #941 it also
    /// travels inside a machine-readable `error.message` that an agent may echo
    /// verbatim. `quilt_rs` is the library crate and ships no binary.
    #[test]
    fn login_required_hint_names_the_binary_not_the_crate() {
        let message = Error::LoginRequired(fixtures::host()).to_string();

        assert!(
            message.contains("> quilt login --host quilt.test"),
            "hint should name the `quilt` binary: {message}"
        );
        assert!(
            !message.contains("quilt_rs login"),
            "hint must not name the library crate: {message}"
        );
    }

    /// Nothing else pins these strings, and a consumer branching on them
    /// cannot see a rename. This table is the contract.
    #[test]
    fn error_kinds_are_stable() {
        let cases: Vec<(Error, &str)> = vec![
            (Error::Domain, "domain"),
            (Error::Home, "home"),
            (
                Error::HomeInUse {
                    reason: "busy".to_string(),
                    retry: home::Retry::OverwriteFlag,
                },
                "home_in_use",
            ),
            (Error::HomeTwice, "home_twice"),
            (Error::CargoMissing, "cargo_missing"),
            (
                Error::UpgradeFailed(upgrade::Installer::Install),
                "upgrade_failed",
            ),
            (
                Error::NotBuilt(upgrade::Installer::Install),
                "upgrade_failed",
            ),
            (
                Error::UpgradeUnsupported(upgrade::Installer::Binstall),
                "upgrade_unsupported",
            ),
            (Error::NamespaceRequired, "namespace_required"),
            (Error::WorkflowEmpty, "workflow_empty"),
            (Error::WorkflowRequiresBucket, "workflow_requires_bucket"),
            (
                Error::CommitMetaInvalid("[]".to_string()),
                "commit_meta_invalid",
            ),
            (Error::Test("probe".to_string()), "internal"),
            (
                Error::NamespaceNotFound(("demo", "sales").into()),
                "namespace_not_found",
            ),
            (Error::LoginRequired(fixtures::host()), "login_required"),
            (
                ReadAllError::from(quilt_rs::dataspace::contract::ContractError::Default {
                    key: "registry",
                    reason: "bad".to_string(),
                })
                .into(),
                "contract",
            ),
            (
                ReadAllError::from(quilt_rs::dataspace::lock::LockError::Identity("host name"))
                    .into(),
                "lock",
            ),
            (
                ReadAllError::Mismatch(vec![quilt_rs::dataspace::lock::Mismatch::Host]).into(),
                "lock_mismatch",
            ),
            (Error::NoCatalog, "no_catalog"),
            (Error::SettingsDir, "settings_dir"),
            (
                Error::Json(
                    serde_json::from_str::<serde_json::Value>("{").expect_err("malformed JSON"),
                ),
                "invalid_json",
            ),
            (Error::Io(std::io::Error::other("boom")), "io"),
            (
                Error::Quilt(quilt_rs::Error::Uri(quilt_uri::UriError::Package(
                    "bad".to_string(),
                ))),
                "invalid_uri",
            ),
            (
                Error::Quilt(quilt_rs::Error::Lineage(quilt_rs::LineageError::Missing)),
                "lineage",
            ),
            (
                Error::Quilt(quilt_rs::Error::Auth(
                    fixtures::host(),
                    quilt_rs::AuthError::TokensRead("boom".to_string()),
                )),
                "auth",
            ),
            (
                Error::Quilt(quilt_rs::Error::Login(quilt_rs::LoginError::NoSession(
                    None,
                ))),
                "login",
            ),
            (
                Error::Quilt(quilt_rs::Error::WorkflowValidation(
                    quilt_rs::WorkflowValidationError::Rejected(
                        quilt_rs::workflow::RuleViolation::WorkflowRequired.into(),
                    ),
                )),
                "workflow_validation",
            ),
            (Error::Quilt(quilt_rs::Error::Unimplemented), "quilt_error"),
        ];

        for (err, expected) in cases {
            assert_eq!(err.kind(), expected, "kind for {err:?}");
        }
    }

    /// A failed upload has its cause's kind.
    #[test]
    fn a_failed_upload_has_its_causes_kind() {
        let err = Error::Quilt(quilt_rs::Error::Upload {
            key: quilt_rs::LogicalKey::new("b.txt").expect("valid key"),
            source: Box::new(quilt_rs::Error::Login(quilt_rs::LoginError::NoSession(
                None,
            ))),
        });
        assert_eq!(err.kind(), "login");
    }

    /// [`error_kinds_are_stable`] for `publish`'s refusal of another's lock.
    #[test]
    fn publish_error_kinds_are_stable() {
        let mismatch = PublishAllError::Mismatch(vec![quilt_rs::dataspace::lock::Mismatch::Host]);
        assert_eq!(Error::from(mismatch).kind(), "lock_mismatch");
    }

    /// [`error_kinds_are_stable`] for `reattach`'s own refusals.
    #[test]
    fn reattach_error_kinds_are_stable() {
        for (err, expected) in [
            (ReattachError::Undeclared, "not_declared"),
            (ReattachError::Attached, "commits_attached"),
            (
                ReattachError::NotUnattached("1".to_string()),
                "not_unattached",
            ),
            (ReattachError::Base("1".to_string()), "invalid_base"),
        ] {
            assert_eq!(Error::from(err).kind(), expected);
        }
    }

    #[test]
    fn commit_workflow_intent_omit_maps_to_bucket_default() {
        assert_eq!(
            commit_workflow_intent(None, false).unwrap(),
            WorkflowIntent::BucketDefault
        );
    }

    #[test]
    fn commit_workflow_intent_no_workflow_flag_opts_out() {
        assert_eq!(
            commit_workflow_intent(None, true).unwrap(),
            WorkflowIntent::NoWorkflow
        );
    }

    #[test]
    fn commit_workflow_intent_named_id_maps_to_named() {
        assert_eq!(
            commit_workflow_intent(Some("x"), false).unwrap(),
            WorkflowIntent::Named("x".to_string())
        );
    }

    #[test]
    fn commit_workflow_intent_blank_id_is_rejected() {
        assert!(matches!(
            commit_workflow_intent(Some(""), false),
            Err(Error::WorkflowEmpty)
        ));
        assert!(matches!(
            commit_workflow_intent(Some("   "), false),
            Err(Error::WorkflowEmpty)
        ));
    }

    /// `--set` is what separates switching from listing, and dropping it
    /// would silently degrade `quilt role --set X` into a plain listing.
    #[test]
    fn role_set_flag_is_parsed() {
        let listing = Args::try_parse_from(["quilt", "role", "--host", "quilt.test"]).unwrap();
        assert!(matches!(listing.command, Commands::Role { set: None, .. }));

        let switching =
            Args::try_parse_from(["quilt", "role", "--host", "quilt.test", "--set", "ReadOnly"])
                .unwrap();
        let Commands::Role { host, set } = switching.command else {
            panic!("expected the role command");
        };
        assert_eq!(host.unwrap().to_string(), "quilt.test");
        assert_eq!(set.as_deref(), Some("ReadOnly"));
    }

    /// A commit is never attached without its base, or `--unpublished` for
    /// none, to check its publish against, and neither names anything
    /// without a commit.
    #[test]
    fn reattach_takes_a_commit_and_its_base_together() {
        let package = "quilt+s3://b#package=team/raw";
        let parse = |rest: &[&str]| {
            Args::try_parse_from(["quilt", "reattach", "ds", package].iter().chain(rest))
        };
        assert!(parse(&[]).is_ok());
        assert!(parse(&["abc", "--base", "def"]).is_ok());
        let unpublished = parse(&["abc", "--unpublished"]).unwrap();
        assert!(matches!(
            unpublished.command,
            Commands::Reattach {
                commit: Some(_),
                base: None,
                unpublished: true,
                ..
            }
        ));
        assert!(parse(&["abc"]).is_err());
        assert!(parse(&["--base", "def"]).is_err());
        assert!(parse(&["--unpublished"]).is_err());
        assert!(parse(&["abc", "--base", "def", "--unpublished"]).is_err());
    }

    /// `--host` is optional on `login` and `role`: left out, they use the
    /// default catalog. `catalog` alone shows it.
    #[test]
    fn host_is_optional_where_a_default_can_stand_in() {
        let login = Args::try_parse_from(["quilt", "login"]).unwrap();
        assert!(matches!(login.command, Commands::Login { host: None, .. }));
        let role = Args::try_parse_from(["quilt", "role"]).unwrap();
        assert!(matches!(role.command, Commands::Role { host: None, .. }));

        let show = Args::try_parse_from(["quilt", "catalog"]).unwrap();
        assert!(matches!(show.command, Commands::Catalog { action: None }));
        let set = Args::try_parse_from(["quilt", "catalog", "set", "quilt.test"]).unwrap();
        let Commands::Catalog {
            action: Some(CatalogAction::Set { host }),
        } = set.command
        else {
            panic!("expected catalog set");
        };
        assert_eq!(host.to_string(), "quilt.test");
        let clear = Args::try_parse_from(["quilt", "catalog", "clear"]).unwrap();
        assert!(matches!(
            clear.command,
            Commands::Catalog {
                action: Some(CatalogAction::Clear)
            }
        ));
        assert!(Args::try_parse_from(["quilt", "catalog", "set"]).is_err());
    }

    /// `--host` names the Quilt stack for a first push, as in `login` and
    /// `role`, and does it without a warning.
    #[test]
    fn push_host_flag_is_parsed() -> Result<(), Error> {
        let args = Args::try_parse_from([
            "quilt",
            "push",
            "--namespace",
            "foo/bar",
            "--bucket",
            "some-bucket",
            "--host",
            "quilt.test",
        ])
        .unwrap();
        let Commands::Push { host, origin, .. } = args.command else {
            panic!("expected the push command");
        };
        let mut stderr = Vec::new();
        let chosen = push_host(host, origin, &mut stderr)?;
        assert_eq!(chosen.unwrap().to_string(), "quilt.test");
        assert_eq!(String::from_utf8(stderr).unwrap(), "");
        Ok(())
    }

    /// A first push may name a bucket without a catalog, and the push then
    /// sets the remote with no host.
    #[test]
    fn push_bucket_without_host_is_parsed() -> Result<(), Error> {
        let args = Args::try_parse_from([
            "quilt",
            "push",
            "--namespace",
            "foo/bar",
            "--bucket",
            "some-bucket",
        ])
        .unwrap();
        let Commands::Push {
            bucket,
            host,
            origin,
            ..
        } = args.command
        else {
            panic!("expected the push command");
        };
        assert_eq!(bucket.as_deref(), Some("some-bucket"));
        assert!(push_host(host, origin, &mut Vec::new())?.is_none());
        Ok(())
    }

    /// The deprecated `--origin` and `-o` still name the stack, with a warning.
    #[test]
    fn push_origin_flag_still_works_and_warns() -> Result<(), Error> {
        for spelling in ["--origin", "-o"] {
            let args = Args::try_parse_from([
                "quilt",
                "push",
                "--namespace",
                "foo/bar",
                "--bucket",
                "some-bucket",
                spelling,
                "quilt.test",
            ])
            .unwrap();
            let Commands::Push { host, origin, .. } = args.command else {
                panic!("expected the push command");
            };
            let mut stderr = Vec::new();
            let chosen = push_host(host, origin, &mut stderr)?;
            assert_eq!(chosen.unwrap().to_string(), "quilt.test");
            assert_eq!(
                String::from_utf8(stderr).unwrap(),
                "warning: --origin is deprecated; use --host\n",
                "{spelling} warns"
            );
        }
        Ok(())
    }

    /// `--host` and the deprecated `--origin` are one flag: passing both is a
    /// usage error rather than a silent pick.
    #[test]
    fn push_host_and_origin_together_are_rejected() {
        let err = Args::try_parse_from([
            "quilt",
            "push",
            "--bucket",
            "some-bucket",
            "--host",
            "one.quilt.test",
            "--origin",
            "another.quilt.test",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    /// A stack needs a bucket, whichever spelling names the stack.
    #[test]
    fn push_host_requires_bucket() {
        for argv in [
            &["quilt", "push", "--host", "quilt.test"][..],
            &["quilt", "push", "--origin", "quilt.test"][..],
        ] {
            let err = Args::try_parse_from(argv).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::MissingRequiredArgument,
                "{argv:?}"
            );
        }
    }

    #[test]
    fn verbose_flag_is_global() {
        let before_subcommand = Args::try_parse_from(["quilt", "--verbose", "list"]).unwrap();
        assert!(before_subcommand.verbose);

        let after_subcommand = Args::try_parse_from(["quilt", "list", "--verbose"]).unwrap();
        assert!(after_subcommand.verbose);

        let default = Args::try_parse_from(["quilt", "list"]).unwrap();
        assert!(!default.verbose);
    }

    fn ours_at(level: &str) -> String {
        format!("quilt={level},quilt_rs={level},quilt_uri={level},quilt_sync={level},warn")
    }

    fn log_flag(argv: &[&str]) -> Result<Option<String>, String> {
        Args::try_parse_with_env(argv)
            .map(|args| args.log_directives())
            .map_err(|err| err.to_string())
    }

    #[test]
    fn log_flag_goes_through_the_shared_rule_on_either_side_of_the_command() {
        assert_eq!(log_flag(&["quilt", "list"]), Ok(None));
        assert_eq!(
            log_flag(&["quilt", "--log", "debug", "list"]),
            Ok(Some(ours_at("debug")))
        );
        assert_eq!(
            log_flag(&["quilt", "list", "--log", "error"]),
            Ok(Some("error".to_string()))
        );
        assert_eq!(
            log_flag(&["quilt", "list", "--log", "quilt_rs=trace,hyper=debug"]),
            Ok(Some("quilt_rs=trace,hyper=debug".to_string()))
        );
    }

    #[test]
    fn verbose_is_log_info() {
        for argv in [["quilt", "-v", "list"], ["quilt", "list", "--verbose"]] {
            assert_eq!(log_flag(&argv), Ok(Some(ours_at("info"))), "{argv:?}");
        }
    }

    /// As `--json` with `--format`: two flags that say the same thing.
    #[test]
    fn verbose_with_log_is_an_error() {
        for argv in [
            ["quilt", "-v", "list", "--log", "debug"],
            ["quilt", "list", "--log", "info", "--verbose"],
        ] {
            let result = log_flag(&argv);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("cannot be used with")),
                "{argv:?}: {result:?}"
            );
        }
    }

    /// An empty `--log`, like `--domain ""`, usually means an unset shell
    /// variable; `quilt_rs=lots` is caught by the strict parse.
    #[test]
    fn a_log_flag_that_does_not_parse_is_an_error() {
        for value in ["", "  ", "debgu", "quilt_rs", "quilt_rs=lots"] {
            let result = log_flag(&["quilt", "list", "--log", value]);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("for '--log <FILTER>'")),
                "{value:?}: {result:?}"
            );
        }
    }

    /// `quilt` with the arguments `line` holds, split at whitespace.
    fn parse(line: &str) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("quilt").chain(line.split_whitespace()))
    }

    /// `commit`, `undo-commit` and `remove-revisions` take `--dataspace` and a
    /// package URI.
    #[test]
    fn the_dataspace_flags_of_commit_undo_and_remove() {
        let uri = "quilt+s3://b#package=a/b";

        let committing = parse(&format!("commit -m x --dataspace ds {uri}"));
        assert!(
            matches!(
                committing.as_ref().map(|args| &args.command),
                Ok(Commands::Commit {
                    dataspace: Some(_),
                    package: Some(_),
                    ..
                })
            ),
            "{committing:?}"
        );
        assert!(parse("commit -m x --dataspace ds -n a/b").is_ok());
        let undoing = parse(&format!("undo-commit --dataspace ds {uri}"));
        assert!(
            matches!(
                undoing.as_ref().map(|args| &args.command),
                Ok(Commands::UndoCommit {
                    dataspace: Some(_),
                    package: Some(_),
                    ..
                })
            ),
            "{undoing:?}"
        );
        assert!(parse("undo-commit -n a/b").is_ok(), "the ref path");
        let removing = parse(&format!(
            "remove-revisions --dataspace ds --revision abc --revision def {uri}"
        ));
        let Ok(Commands::RemoveRevisions { revision, .. }) = removing.map(|args| args.command)
        else {
            panic!("remove-revisions --dataspace did not parse");
        };
        assert_eq!(revision, ["abc", "def"]);
    }

    /// A package URI needs `--dataspace`, and the ref path's own flags refuse
    /// it.
    #[test]
    fn the_dataspace_flags_refuse_what_the_ref_path_takes() {
        let uri = "quilt+s3://b#package=a/b";
        for refused in [
            format!("commit -m x {uri}"),
            "commit -m x --dataspace ds not-a-package-uri".to_string(),
            format!("commit -m x --dataspace ds --meta {{}} {uri}"),
            format!("commit -m x --dataspace ds --no-workflow {uri}"),
            format!("commit -m x --dataspace ds -n a/b {uri}"),
            format!("undo-commit {uri}"),
            format!("remove-revisions --dataspace ds {uri}"),
            "remove-revisions --revision abc -n a/b".to_string(),
            format!("remove-revisions --dataspace ds --revision abc --count 1 {uri}"),
        ] {
            assert!(parse(&refused).is_err(), "{refused:?}");
        }
    }

    /// `reset` and `certify` act on a dataspace only.
    #[test]
    fn reset_and_certify_need_a_dataspace() {
        for verb in ["reset", "certify"] {
            let named = parse(&format!("{verb} --dataspace ds -n a/b"));
            assert!(named.is_ok(), "{verb}: {named:?}");
            let keyed = parse(&format!("{verb} --dataspace ds quilt+s3://b#package=a/b"));
            assert!(keyed.is_ok(), "{verb}: {keyed:?}");
            let alone = parse(&format!("{verb} -n a/b"));
            assert!(alone.is_err(), "{verb}: {alone:?}");
        }
    }

    /// `--prune` is off unless given.
    #[test]
    fn uninstall_prunes_only_when_asked() {
        let plain = Args::try_parse_from(["quilt", "uninstall", "-n", "a/b"]).expect("parses");
        assert!(matches!(
            plain.command,
            Commands::Uninstall { prune: false, .. }
        ));
        let pruning =
            Args::try_parse_from(["quilt", "uninstall", "-n", "a/b", "--prune"]).expect("parses");
        assert!(matches!(
            pruning.command,
            Commands::Uninstall { prune: true, .. }
        ));
    }

    /// `--count` is optional, and refuses 0 as an argument error.
    #[test]
    fn keep_takes_one_scope_and_no_download_needs_one() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt", "keep"], args].concat());
        assert!(matches!(
            parse(&[]).expect("parses").command,
            Commands::Keep { files: false, all: false, ref folder, no_download: false, .. }
                if folder.is_empty()
        ));
        assert!(matches!(
            parse(&["--folder", "a", "--folder", "b/"]).expect("parses").command,
            Commands::Keep { ref folder, .. } if folder == &["a", "b/"]
        ));
        assert!(matches!(
            parse(&["--all", "--no-download"]).expect("parses").command,
            Commands::Keep {
                all: true,
                no_download: true,
                ..
            }
        ));
        assert!(parse(&["--all", "--files"]).is_err());
        assert!(parse(&["--all", "--folder", "a"]).is_err());
        assert!(parse(&["--no-download"]).is_err());
    }

    #[test]
    fn install_takes_one_keep_flag_and_no_download_needs_one() {
        let parse = |args: &[&str]| {
            Args::try_parse_from([&["quilt", "install", "quilt+s3://b#package=a/b"], args].concat())
        };
        assert!(matches!(
            parse(&["--keep-folder", "a", "--keep-folder", "b"]).expect("parses").command,
            Commands::Install { ref keep_folder, keep_all: false, .. } if keep_folder == &["a", "b"]
        ));
        assert!(matches!(
            parse(&["--keep-all", "--no-download"])
                .expect("parses")
                .command,
            Commands::Install {
                keep_all: true,
                no_download: true,
                ..
            }
        ));
        assert!(parse(&["--keep-all", "--keep-folder", "a"]).is_err());
        assert!(parse(&["--no-download"]).is_err());
    }

    /// `--dataspace` takes an access, which needs it, and none of the home's
    /// flags.
    #[test]
    fn install_into_a_dataspace_takes_an_access_and_no_home_flags() {
        let parse = |args: &[&str]| {
            Args::try_parse_from([&["quilt", "install", "quilt+s3://b#package=a/b"], args].concat())
        };
        assert!(matches!(
            parse(&["--dataspace", "ds", "--access", "read", "--path", "x/"])
                .expect("parses")
                .command,
            Commands::Install {
                dataspace: Some(_),
                access: Some(Access::Read),
                path: Some(_),
                ..
            }
        ));
        assert!(matches!(
            parse(&["--dataspace", "ds"]).expect("parses").command,
            Commands::Install { access: None, .. }
        ));
        assert!(parse(&["--access", "read"]).is_err());
        assert!(parse(&["--dataspace", "ds", "--access", "all"]).is_err());
        assert!(parse(&["--dataspace", "ds", "--keep-all"]).is_err());
        assert!(parse(&["--dataspace", "ds", "-n", "a/b"]).is_err());
    }

    /// A package URI names what to uninstall from a dataspace, which it
    /// needs; `-n` names it instead.
    #[test]
    fn uninstall_from_a_dataspace_takes_a_package_or_a_namespace() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt", "uninstall"], args].concat());
        assert!(matches!(
            parse(&["--dataspace", "ds", "quilt+s3://b#package=a/b"])
                .expect("parses")
                .command,
            Commands::Uninstall {
                dataspace: Some(_),
                package: Some(_),
                ..
            }
        ));
        assert!(parse(&["--dataspace", "ds", "-n", "a/b"]).is_ok());
        assert!(parse(&["quilt+s3://b#package=a/b"]).is_err());
        assert!(parse(&["--dataspace", "ds", "quilt+s3://b#package=a/b", "-n", "a/b"]).is_err());
    }

    /// `--dataspace` turns `status`, `list` and `log` to a dataspace:
    /// `status` then takes no namespace or `--ignored`, and `log` takes the
    /// package by its URI, which needs the dataspace.
    #[test]
    fn status_list_and_log_take_a_dataspace() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt"], args].concat());
        let uri = "quilt+s3://b#package=a/b";
        assert!(matches!(
            parse(&["status", "--dataspace", "ds"])
                .expect("parses")
                .command,
            Commands::Status {
                dataspace: Some(_),
                ..
            }
        ));
        assert!(parse(&["status", "--dataspace", "ds", "-n", "a/b"]).is_err());
        assert!(parse(&["status", "--dataspace", "ds", "--ignored"]).is_err());
        assert!(matches!(
            parse(&["list", "--dataspace", "ds", "--fetch"])
                .expect("parses")
                .command,
            Commands::List {
                dataspace: Some(_),
                fetch: true
            }
        ));
        assert!(matches!(
            parse(&["log", "--dataspace", "ds", uri])
                .expect("parses")
                .command,
            Commands::Log {
                dataspace: Some(_),
                package: Some(_),
                ..
            }
        ));
        assert!(parse(&["log", uri]).is_err());
        assert!(parse(&["log", "--dataspace", "ds"]).is_err());
        assert!(parse(&["log", "--dataspace", "ds", uri, "-n", "a/b"]).is_err());
    }

    /// `keep --dataspace` takes paths, a pin and a list, none of which the
    /// home's keep takes, and none of the home's `--files` or
    /// `--no-download`.
    #[test]
    fn keep_in_a_dataspace_takes_paths_a_pin_and_a_list() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt", "keep"], args].concat());
        assert!(matches!(
            parse(&[
                "--dataspace",
                "ds",
                "quilt+s3://b#package=a/b",
                "--folder",
                "data",
                "--path",
                "a.txt",
                "--pin",
                "abcdef1234",
                "--access",
                "read",
                "--from",
                "write",
            ])
            .expect("parses")
            .command,
            Commands::Keep {
                dataspace: Some(_),
                package: Some(_),
                pin: Some(_),
                unpin: false,
                access: Some(Access::Read),
                from: Some(Access::Write),
                ..
            }
        ));
        assert!(parse(&["--dataspace", "ds", "-n", "a/b", "--unpin"]).is_ok());
        assert!(parse(&["--dataspace", "ds", "-n", "a/b", "--all", "--path", "x"]).is_err());
        assert!(parse(&["--dataspace", "ds", "-n", "a/b", "--pin", "a", "--unpin"]).is_err());
        assert!(parse(&["--dataspace", "ds", "-n", "a/b", "--files"]).is_err());
        assert!(parse(&["--dataspace", "ds", "-n", "a/b", "--all", "--no-download"]).is_err());
        assert!(parse(&["--dataspace", "ds", "quilt+s3://b#package=a/b@abc"]).is_err());
        assert!(parse(&["-n", "a/b", "--pin", "abc"]).is_err());
        assert!(parse(&["-n", "a/b", "--path", "x"]).is_err());
        assert!(parse(&["quilt+s3://b#package=a/b"]).is_err());
    }

    /// `create --dataspace` takes the package's URI and an access, and no
    /// `--meta`; without it, `-n` is still required.
    #[test]
    fn create_in_a_dataspace_takes_a_uri_and_an_access() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt", "create"], args].concat());
        assert!(matches!(
            parse(&[
                "--dataspace",
                "ds",
                "quilt+local://#package=me/analysis",
                "--access",
                "write",
                "--source",
                "/tmp/analysis",
            ])
            .expect("parses")
            .command,
            Commands::Create {
                dataspace: Some(_),
                package: Some(_),
                access: Some(Access::Write),
                source: Some(_),
                namespace: None,
                ..
            }
        ));
        assert!(parse(&["--dataspace", "ds"]).is_err());
        assert!(parse(&["quilt+s3://b#package=a/b"]).is_err());
        assert!(
            parse(&[
                "--dataspace",
                "ds",
                "quilt+s3://b#package=a/b",
                "--meta",
                "{}"
            ])
            .is_err()
        );
        assert!(parse(&["-n", "a/b", "--access", "sync"]).is_err());
        assert!(parse(&[]).is_err());
        assert!(matches!(
            parse(&["-n", "a/b"]).expect("parses").command,
            Commands::Create {
                namespace: Some(_),
                dataspace: None,
                ..
            }
        ));
    }

    /// `gc --dataspace` takes `--dry-run`, which needs it; `home --dataspace`
    /// takes a folder and no `--overwrite`; `storage` and `local-registry`
    /// are verbs of their own.
    #[test]
    fn the_domain_verbs_parse() {
        let parse = |args: &[&str]| Args::try_parse_from([&["quilt"], args].concat());
        assert!(matches!(
            parse(&["gc", "--dataspace", "--dry-run"])
                .expect("parses")
                .command,
            Commands::Gc {
                dataspace: true,
                dry_run: true
            }
        ));
        assert!(parse(&["gc", "--dry-run"]).is_err());
        assert!(matches!(
            parse(&["gc"]).expect("parses").command,
            Commands::Gc {
                dataspace: false,
                ..
            }
        ));
        assert!(matches!(
            parse(&["home", "--dataspace", "ds"])
                .expect("parses")
                .command,
            Commands::Home {
                dir: Some(_),
                dataspace: true,
                overwrite: false
            }
        ));
        assert!(parse(&["home", "--dataspace", "ds", "--overwrite"]).is_err());
        assert!(matches!(
            parse(&["storage"]).expect("parses").command,
            Commands::Storage
        ));
        assert!(matches!(
            parse(&["local-registry", "lab-mac"])
                .expect("parses")
                .command,
            Commands::LocalRegistry { name: Some(_) }
        ));
        assert!(matches!(
            parse(&["local-registry"]).expect("parses").command,
            Commands::LocalRegistry { name: None }
        ));
    }

    #[test]
    fn remove_revisions_count_is_optional_and_at_least_one() {
        let all = Args::try_parse_from(["quilt", "remove-revisions", "-n", "a/b"]).expect("parses");
        assert!(matches!(
            all.command,
            Commands::RemoveRevisions { count: None, .. }
        ));
        let two = Args::try_parse_from(["quilt", "remove-revisions", "-n", "a/b", "-c", "2"])
            .expect("parses");
        assert!(matches!(
            two.command,
            Commands::RemoveRevisions { count: Some(count), .. } if count.get() == 2
        ));
        let err = Args::try_parse_from(["quilt", "remove-revisions", "--count", "0"])
            .expect_err("0 is refused");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(err.to_string().contains("must be at least 1"), "{err}");
    }

    /// `global = true` is what makes this additive: the shipped spelling keeps
    /// working and the new one starts working.
    #[test]
    fn json_flag_parses_before_or_after_the_subcommand() {
        let after = Args::try_parse_from(["quilt", "list", "--json"]).expect("parses");
        assert!(after.json);
        assert!(matches!(after.command, Commands::List { fetch: false, .. }));

        let before = Args::try_parse_from(["quilt", "--json", "list"]).expect("parses");
        assert!(before.json);
        assert!(matches!(
            before.command,
            Commands::List { fetch: false, .. }
        ));

        let status = Args::try_parse_from(["quilt", "status", "-n", "demo/sales", "--json"])
            .expect("parses");
        assert!(status.json);

        let default = Args::try_parse_from(["quilt", "list"]).expect("parses");
        assert!(!default.json);
    }

    /// `global = true` keeps the shipped spelling working and adds the other.
    #[test]
    fn domain_flag_parses_before_or_after_the_subcommand() {
        let domain = Some(PathBuf::from("/tmp/quilt-domain"));

        for argv in [
            ["quilt", "--domain", "/tmp/quilt-domain", "list"],
            ["quilt", "-d", "/tmp/quilt-domain", "list"],
            ["quilt", "list", "--domain", "/tmp/quilt-domain"],
            ["quilt", "list", "-d", "/tmp/quilt-domain"],
        ] {
            let args = Args::try_parse_from(argv).expect("parses");
            assert_eq!(args.domain, domain, "{argv:?}");
            assert!(
                matches!(args.command, Commands::List { fetch: false, .. }),
                "{argv:?}"
            );
        }

        let status = Args::try_parse_from([
            "quilt",
            "status",
            "-n",
            "demo/sales",
            "--domain",
            "/tmp/quilt-domain",
        ])
        .expect("parses");
        assert_eq!(status.domain, domain);
        assert!(matches!(
            status.command,
            Commands::Status {
                pkg: PackageRef { namespace: Some(namespace) },
                ignored: false,
                dataspace: None,
            } if namespace == "demo/sales"
        ));
    }

    /// What a child parse resolved to: the domain and the result's format, or
    /// the parse error's message.
    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Parsed {
        domain: Result<PathBuf, String>,
        json: bool,
        /// Where `quilt version` says the domain came from.
        domain_from: Option<String>,
    }

    /// Parses `argv` with each named variable set to its value (or removed,
    /// for `None`) and returns what it resolves to, or the parse error.
    ///
    /// Clap reads the variables from the process environment, and setting
    /// them here would race the other tests. So the parse runs in a child:
    /// this test binary re-run on `env_child` alone, with the variables set
    /// only in the child's environment.
    fn parse_under_env(argv: &[&str], vars: &[(&str, Option<&str>)]) -> Result<Parsed, String> {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["cli::tests::env_child", "--exact", "--nocapture"])
            .env(ENV_CHILD_ARGV, argv.join("\u{1f}"))
            // The ref path's parsing, which the switch off keeps.
            .env(switch::SWITCH_ENV, "0");
        for env in [DOMAIN_ENV, FORMAT_ENV] {
            match vars.iter().find(|(name, _)| *name == env) {
                Some((_, Some(value))) => child.env(env, value),
                _ => child.env_remove(env),
            };
        }
        let output = child.output().expect("spawn the parser");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout
            .lines()
            .find_map(|line| line.strip_prefix(ENV_CHILD_MARKER))
            .unwrap_or_else(|| panic!("the child printed no result: {stdout}"));
        serde_json::from_str::<Result<Parsed, String>>(line).expect("the child's result")
    }

    /// `parse_under_env` with `QUILT_DOMAIN` alone, returning the domain.
    fn parse_domain_under_env(argv: &[&str], value: Option<&str>) -> Result<PathBuf, String> {
        parse_under_env(argv, &[(DOMAIN_ENV, value)]).and_then(|parsed| parsed.domain)
    }

    /// `parse_under_env` with `QUILT_FORMAT` alone, returning the format.
    fn parse_format_under_env(argv: &[&str], value: Option<&str>) -> Result<Format, String> {
        parse_under_env(argv, &[(FORMAT_ENV, value)]).map(|parsed| {
            if parsed.json {
                Format::Json
            } else {
                Format::Text
            }
        })
    }

    /// A domain's first command leaves `data.json` the stub an older quilt
    /// fails on, and the package in its own ref.
    #[test(tokio::test)]
    async fn a_new_domain_keeps_its_lineage_in_refs_behind_the_stub() -> Result<(), Error> {
        let domain = tempfile::TempDir::new()?;
        let root = domain.path().to_string_lossy().to_string();
        let args = Args::try_parse_from(["quilt", "--domain", &root, "create", "-n", "acme/x"])
            .map_err(|err| Error::Test(err.to_string()))?;

        init(args).await?;

        let dot_quilt = domain.path().join(".quilt");
        let data_json = std::fs::read(dot_quilt.join("data.json"))?;
        let stub: serde_json::Value = serde_json::from_slice(&data_json)?;
        assert_eq!(stub["lineage"], "refs", "{stub}");
        assert!(quilt_rs::lineage::DomainLineage::from_slice(&data_json).is_err());
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dot_quilt.join("domain.json"))?)?;
        assert_eq!(config["migration"], "done", "{config}");
        assert!(dot_quilt.join("refs/acme/x.json").is_file());
        Ok(())
    }

    const ENV_CHILD_ARGV: &str = "QUILT_TEST_ENV_ARGV";
    const ENV_CHILD_MARKER: &str = "env-child-result:";

    /// The child half of `parse_under_env`; a no-op when run directly.
    #[test]
    fn env_child() {
        let Ok(argv) = std::env::var(ENV_CHILD_ARGV) else {
            return;
        };
        let result = Args::try_parse_with_env(argv.split('\u{1f}'))
            .map_err(|err| err.to_string())
            .map(|args| Parsed {
                json: args.format() == Format::Json,
                domain_from: match &args.command {
                    Commands::Version { domain_from, .. } => Some(domain_from.to_string()),
                    _ => None,
                },
                domain: get_domain_dir(args.domain).map_err(|err| err.to_string()),
            });
        println!(
            "{ENV_CHILD_MARKER}{}",
            serde_json::to_string(&result).unwrap()
        );
    }

    #[test]
    fn domain_env_is_used_without_the_flag() {
        let domain = PathBuf::from("/tmp/quilt-env-domain");
        assert_eq!(
            parse_domain_under_env(&["quilt", "list"], Some("/tmp/quilt-env-domain")),
            Ok(domain.clone())
        );
        // Global, like the flag: the variable reaches every command.
        assert_eq!(
            parse_domain_under_env(
                &["quilt", "status", "-n", "demo/sales"],
                Some("/tmp/quilt-env-domain")
            ),
            Ok(domain)
        );
    }

    #[test]
    fn version_says_where_the_domain_came_from() {
        let from = |argv: &[&str], value: Option<&str>| {
            parse_under_env(argv, &[(DOMAIN_ENV, value)])
                .map(|parsed| parsed.domain_from)
                .expect("parses")
        };
        let version = ["quilt", "version", "--details"];
        assert_eq!(from(&version, None).as_deref(), Some("default"));
        assert_eq!(from(&version, Some("")).as_deref(), Some("default"));
        assert_eq!(
            from(&version, Some("/tmp/quilt-env-domain")).as_deref(),
            Some("QUILT_DOMAIN")
        );
        for argv in [
            [
                "quilt",
                "version",
                "--details",
                "-d",
                "/tmp/quilt-flag-domain",
            ],
            [
                "quilt",
                "--domain",
                "/tmp/quilt-flag-domain",
                "version",
                "--details",
            ],
        ] {
            assert_eq!(
                from(&argv, Some("/tmp/quilt-env-domain")).as_deref(),
                Some("--domain"),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn domain_flag_beats_domain_env() {
        let flag = PathBuf::from("/tmp/quilt-flag-domain");
        for argv in [
            ["quilt", "--domain", "/tmp/quilt-flag-domain", "list"],
            ["quilt", "list", "-d", "/tmp/quilt-flag-domain"],
        ] {
            assert_eq!(
                parse_domain_under_env(&argv, Some("/tmp/quilt-env-domain")),
                Ok(flag.clone()),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn empty_domain_env_falls_back_to_the_default() {
        let default = get_domain_dir(None).map_err(|err| err.to_string());
        assert_eq!(parse_domain_under_env(&["quilt", "list"], None), default);
        assert_eq!(
            parse_domain_under_env(&["quilt", "list"], Some("")),
            default
        );
        // An empty variable leaves the flag in charge.
        assert_eq!(
            parse_domain_under_env(&["quilt", "list", "-d", "/tmp/quilt-flag-domain"], Some("")),
            Ok(PathBuf::from("/tmp/quilt-flag-domain"))
        );
    }

    /// Only an empty variable means unset: `--domain ""` is a mistake, such as
    /// an unset shell variable, and must not quietly pick the default domain.
    #[test]
    fn empty_domain_flag_is_an_error() {
        for value in [None, Some(""), Some("/tmp/quilt-env-domain")] {
            let result = parse_domain_under_env(&["quilt", "list", "--domain", ""], value);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("a value is required for '--domain")),
                "{value:?}: {result:?}"
            );
        }
    }

    /// The hint shows whatever the variable holds, an empty value included.
    #[test]
    fn help_names_the_domain_env() {
        for value in [None, Some(""), Some("/tmp/quilt-env-domain")] {
            // `--help` is a parse "error" whose message is the help text.
            let help = parse_domain_under_env(&["quilt", "--help"], value).unwrap_err();
            assert!(help.contains("[env: QUILT_DOMAIN"), "{value:?}: {help}");
        }
    }

    /// The example the change was agreed on, line by line.
    #[test]
    fn format_example() {
        assert_eq!(
            parse_format_under_env(&["quilt", "list"], Some("json")),
            Ok(Format::Json)
        );
        assert_eq!(
            parse_format_under_env(&["quilt", "list", "--format", "text"], Some("json")),
            Ok(Format::Text)
        );
        assert_eq!(
            parse_format_under_env(&["quilt", "list", "--json"], None),
            Ok(Format::Json)
        );
        let conflict =
            parse_format_under_env(&["quilt", "list", "--json", "--format", "text"], None);
        assert!(
            conflict
                .as_ref()
                .is_err_and(|err| err.contains("cannot be used with")),
            "{conflict:?}"
        );
    }

    #[test]
    fn format_defaults_to_text() {
        assert_eq!(
            parse_format_under_env(&["quilt", "list"], None),
            Ok(Format::Text)
        );
    }

    #[test]
    fn format_env_is_used_without_the_flag() {
        for (value, format) in [("json", Format::Json), ("text", Format::Text)] {
            assert_eq!(
                parse_format_under_env(&["quilt", "list"], Some(value)),
                Ok(format)
            );
            // Global, like the flag: the variable reaches every command.
            assert_eq!(
                parse_format_under_env(&["quilt", "status", "-n", "demo/sales"], Some(value)),
                Ok(format)
            );
        }
    }

    #[test]
    fn format_flag_beats_format_env() {
        for (argv, env, format) in [
            (["quilt", "--format", "json", "list"], "text", Format::Json),
            (["quilt", "list", "--format", "json"], "text", Format::Json),
            (["quilt", "--format", "text", "list"], "json", Format::Text),
            (["quilt", "list", "--format", "text"], "json", Format::Text),
        ] {
            assert_eq!(
                parse_format_under_env(&argv, Some(env)),
                Ok(format),
                "{argv:?} with {env}"
            );
        }
    }

    /// `--json` is shorthand for `--format json`, on either side of the verb.
    #[test]
    fn json_flag_still_parses_before_and_after_the_command() {
        for argv in [["quilt", "--json", "list"], ["quilt", "list", "--json"]] {
            assert_eq!(
                parse_format_under_env(&argv, None),
                Ok(Format::Json),
                "{argv:?}"
            );
        }
    }

    /// A flag beats the variable, and `--json` is a flag: it is no conflict
    /// with `QUILT_FORMAT=text`, only with `--format`.
    #[test]
    fn json_flag_beats_format_env() {
        for argv in [["quilt", "--json", "list"], ["quilt", "list", "--json"]] {
            assert_eq!(
                parse_format_under_env(&argv, Some("text")),
                Ok(Format::Json),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn json_flag_with_format_flag_is_an_error() {
        for argv in [
            ["quilt", "list", "--json", "--format", "text"],
            ["quilt", "--format", "text", "list", "--json"],
            ["quilt", "list", "--json", "--format", "json"],
        ] {
            let result = parse_format_under_env(&argv, None);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("cannot be used with")),
                "{argv:?}: {result:?}"
            );
        }
    }

    #[test]
    fn empty_format_env_falls_back_to_text() {
        assert_eq!(
            parse_format_under_env(&["quilt", "list"], Some("")),
            Ok(Format::Text)
        );
        // An empty variable leaves the flags in charge.
        assert_eq!(
            parse_format_under_env(&["quilt", "list", "--format", "json"], Some("")),
            Ok(Format::Json)
        );
        assert_eq!(
            parse_format_under_env(&["quilt", "list", "--json"], Some("")),
            Ok(Format::Json)
        );
    }

    /// As with `--domain ""`, only an empty variable means unset.
    #[test]
    fn empty_format_flag_is_an_error() {
        for value in [None, Some(""), Some("json")] {
            let result = parse_format_under_env(&["quilt", "list", "--format", ""], value);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("a value is required for '--format")),
                "{value:?}: {result:?}"
            );
        }
    }

    /// An unknown value is an error, never a quiet fall back to text.
    #[test]
    fn unknown_format_is_an_error() {
        for (argv, value) in [
            (&["quilt", "list"][..], Some("yaml")),
            (&["quilt", "list", "--json"][..], Some("yaml")),
            (&["quilt", "list", "--format", "yaml"][..], None),
        ] {
            let result = parse_format_under_env(argv, value);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|err| err.contains("invalid value 'yaml' for '--format")),
                "{argv:?} with {value:?}: {result:?}"
            );
        }
    }

    /// The hint shows whatever the variable holds, an empty value included.
    #[test]
    fn help_names_the_format_env() {
        for value in [None, Some(""), Some("json")] {
            let help = parse_format_under_env(&["quilt", "--help"], value).unwrap_err();
            assert!(help.contains("[env: QUILT_FORMAT"), "{value:?}: {help}");
            assert!(help.contains("result"), "{value:?}: {help}");
        }
    }

    #[test]
    fn status_ignored_flag_parses() {
        let args = Args::try_parse_from(["quilt", "status", "--ignored"]).unwrap();
        assert!(matches!(
            args.command,
            Commands::Status { ignored: true, .. }
        ));
    }

    #[test]
    fn package_namespace_flag_is_optional() {
        let inferred = Args::try_parse_from(["quilt", "status"]).unwrap();
        assert!(matches!(
            inferred.command,
            Commands::Status {
                pkg: PackageRef { namespace: None },
                ignored: false,
                dataspace: None,
            }
        ));

        let explicit =
            Args::try_parse_from(["quilt", "status", "--namespace", "demo/sales"]).unwrap();
        assert!(matches!(
            explicit.command,
            Commands::Status {
                pkg: PackageRef { namespace: Some(namespace) },
                ignored: false,
                dataspace: None,
            } if namespace == "demo/sales"
        ));
    }

    #[test]
    fn namespace_from_working_dir_uses_package_components() -> Result<(), Error> {
        let home = Path::new("/home/user/QuiltSync");
        let current_dir = Path::new("/home/user/QuiltSync/demo/sales/src");

        assert_eq!(
            namespace_from_working_dir(home, current_dir)?,
            Namespace::from(("demo", "sales"))
        );
        Ok(())
    }

    #[test]
    fn namespace_from_working_dir_requires_a_package_path() {
        let home = Path::new("/home/user/QuiltSync");

        assert!(matches!(
            namespace_from_working_dir(home, Path::new("/home/user/QuiltSync")),
            Err(Error::NamespaceRequired)
        ));
        assert!(matches!(
            namespace_from_working_dir(home, Path::new("/home/user/other/demo/sales")),
            Err(Error::NamespaceRequired)
        ));
    }

    #[test]
    fn test_parse_optional_namespace() -> Result<(), Error> {
        // Test None case
        assert!(parse_optional_namespace(None)?.is_none());

        // Test Some valid namespace
        let ns = parse_optional_namespace(Some("foo/bar".to_string()))?.unwrap();
        assert_eq!(ns.to_string(), "foo/bar");

        // Test Some invalid namespace
        let err = parse_optional_namespace(Some("invalid".to_string())).unwrap_err();
        assert!(matches!(err, Error::Quilt(_)));

        Ok(())
    }

    #[test]
    fn test_get_domain_dir() -> Result<(), Error> {
        // Test with provided directory
        let test_dir = PathBuf::from("/test/path");
        assert_eq!(get_domain_dir(Some(test_dir.clone()))?, test_dir);

        // Test with None (should use default location)
        if let Some(local_dir) = dirs::data_local_dir() {
            let expected = local_dir.join(DOMAIN_DIR_NAMESPACE);
            assert_eq!(get_domain_dir(None)?, expected);
        } else {
            // If data_local_dir() returns None, get_domain_dir should return Error::Domain
            assert!(matches!(get_domain_dir(None), Err(Error::Domain)));
        }

        Ok(())
    }

    #[test]
    fn default_home_dir_is_quiltsync_under_the_user_home() {
        assert_eq!(
            default_home_dir_under(Some(PathBuf::from("/home/ann"))).unwrap(),
            PathBuf::from("/home/ann/QuiltSync")
        );
        assert!(matches!(default_home_dir_under(None), Err(Error::Home)));
    }

    /// `set_home` creates the folder, so a test that sets the default home
    /// must not reach the real one.
    #[test]
    fn tests_never_use_the_real_default_home() {
        let real = dirs::home_dir().map(|home| home.join(quilt_rs::DEFAULT_HOME_DIR_NAME));
        assert_ne!(get_default_home_dir().ok(), real);
    }

    /// Commands that never touch the home don't set or create one, so a home
    /// folder that can't be created can't block them.
    #[test(tokio::test)]
    async fn commands_without_a_home_leave_a_new_domain_without_one() -> Result<(), Error> {
        let commands = [
            Commands::Login {
                code: None,
                host: Some(fixtures::host()),
            },
            Commands::Browse {
                uri: "not-a-package-uri".to_string(),
            },
        ];
        for command in commands {
            let domain = tempfile::tempdir()?;
            let args = Args {
                home: None,
                domain: Some(domain.path().to_path_buf()),
                verbose: false,
                log: None,
                format: None,
                json: false,
                command,
            };

            let _ = init(args).await;

            let stored = quilt_rs::LocalDomain::new(domain.path())
                .stored_home()
                .await;
            assert!(matches!(stored, Ok(None)), "no home is stored: {stored:?}");
            let dot_quilt = domain.path().join(".quilt");
            assert!(!dot_quilt.join("domain.json").exists(), "no move ran");
            assert!(!dot_quilt.join("data.json").exists(), "no move ran");
        }
        Ok(())
    }

    /// The deprecated `--home` still sets the home, whatever the command.
    #[test(tokio::test)]
    async fn home_flag_still_sets_the_home_before_login() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        let args = Args {
            home: Some(home.path().to_path_buf()),
            domain: Some(domain.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Login {
                code: None,
                host: Some(fixtures::host()),
            },
        };

        init(args).await?;

        let stored = quilt_rs::LocalDomain::new(domain.path()).get_home().await?;
        assert_eq!(stored.as_ref(), home.path());
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_list_uses_default_home_without_flag() -> Result<(), Error> {
        let domain_temp_dir = tempfile::tempdir()?;
        let list_args = Args {
            home: None,
            domain: Some(domain_temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::List {
                fetch: false,
                dataspace: None,
            },
        };

        let mut output = Vec::new();
        let result = init(list_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "No installed packages\n"
        );

        let stored_home = quilt_rs::LocalDomain::new(domain_temp_dir.path())
            .get_home()
            .await?;
        assert_eq!(stored_home.as_ref(), &get_default_home_dir()?);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_missing_home_lineage_is_repaired_without_flag() -> Result<(), Error> {
        let (model, domain_temp_dir) = Model::from_temp_dir()?;
        let paths = quilt_rs::paths::DomainPaths::new(domain_temp_dir.path().to_path_buf());
        quilt_rs::lineage::DomainLineageIo::write_fixture(
            quilt_rs::lineage::LineageStore::DataJson,
            &quilt_rs::io::storage::LocalStorage::new(),
            &paths,
            "",
            [],
        )
        .await?;

        initialize_home(&model, None, &mut Vec::new()).await?;

        assert_eq!(model.get_home().await?.as_ref(), &get_default_home_dir()?);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_existing_home_is_preserved_without_flag() -> Result<(), Error> {
        let (model, _domain_temp_dir) = Model::from_temp_dir()?;
        let home_temp_dir = tempfile::tempdir()?;
        model.set_home(home_temp_dir.path()).await?;

        initialize_home(&model, None, &mut Vec::new()).await?;

        assert_eq!(
            model.get_home().await?.as_ref(),
            &home_temp_dir.path().to_path_buf()
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn live_install() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        // Create temporary directory for domain
        let domain_temp_dir = tempfile::tempdir()?;
        let domain = Some(domain_temp_dir.path().to_path_buf());

        let working_temp_dir = tempfile::tempdir()?;
        let home = Some(working_temp_dir.path().to_path_buf());

        // First install the package
        let install_args = Args {
            home,
            domain,
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Install {
                namespace: Some(Namespace::from(pkg::NAMESPACE).to_string()),
                uri: pkg::URI.to_string(),
                path: None,
                keep_all: false,
                keep_folder: Vec::new(),
                no_download: false,
                dataspace: None,
                access: None,
                folder: None,
            },
        };
        let mut output = Vec::new();
        let result = init(install_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            format!(
                "Installed package \"{}\"\nNo paths installed\n",
                pkg::NAMESPACE_STR,
            )
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_commit_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let commit_args = Args {
            home: Some(temp_dir.path().to_path_buf()),
            domain: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Commit {
                message: pkg::MESSAGE.to_string(),
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
                package: None,
                dataspace: None,
                user_meta: None,
                workflow: None,
                no_workflow: true,
            },
        };

        // The installed revision's own message: the same hash, so nothing to commit
        let mut output = Vec::new();
        let result = init(commit_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Nothing to commit\n".to_string());

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_commit_invalid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let commit_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Commit {
                message: "Any message".to_string(),
                pkg: PackageRef {
                    namespace: Some("in/valid".to_string()),
                },
                package: None,
                dataspace: None,
                user_meta: None,
                workflow: None,
                no_workflow: true,
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(commit_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Package in/valid not found\n".to_string());

        Ok(())
    }

    #[test]
    fn meta_and_user_meta_flags_agree_on_commit_and_create() {
        let json = r#"{"n": 1}"#;
        let commit_flags = [
            ["quilt", "commit", "-m", "x", "--meta", json],
            ["quilt", "commit", "-m", "x", "-u", json],
            ["quilt", "commit", "-m", "x", "--user-meta", json],
        ];
        let metas: Vec<_> = commit_flags
            .iter()
            .map(|argv| {
                let args = Args::try_parse_from(*argv).expect("parses");
                let Commands::Commit { user_meta, .. } = args.command else {
                    panic!("expected commit: {args:?}");
                };
                user_meta
            })
            .collect();
        assert!(metas.iter().all(|m| m.as_deref() == Some(json)));

        let create_flags = [
            ["quilt", "create", "-n", "a/b", "--meta", json],
            ["quilt", "create", "-n", "a/b", "-u", json],
            ["quilt", "create", "-n", "a/b", "--user-meta", json],
        ];
        let metas: Vec<_> = create_flags
            .iter()
            .map(|argv| {
                let args = Args::try_parse_from(*argv).expect("parses");
                let Commands::Create { user_meta, .. } = args.command else {
                    panic!("expected create: {args:?}");
                };
                user_meta
            })
            .collect();
        assert!(metas.iter().all(|m| m.as_deref() == Some(json)));
    }

    #[test]
    fn parse_meta_flag_accepts_an_object_and_refuses_other_json() {
        let object = r#"{"n": 18446744073709551616}"#;
        let UserMeta::Set(value, numbers) =
            parse_meta_flag(Some(object), UserMeta::Keep).expect("object")
        else {
            panic!("expected Set");
        };
        assert!(value["n"].is_number());
        assert!(!numbers.is_empty());

        assert!(matches!(
            parse_meta_flag(None, UserMeta::Keep).unwrap(),
            UserMeta::Keep
        ));
        assert!(matches!(
            parse_meta_flag(None, UserMeta::Clear).unwrap(),
            UserMeta::Clear
        ));

        for rejected in ["null", "[]", "1", "\"s\"", "true"] {
            let err = parse_meta_flag(Some(rejected), UserMeta::Keep).unwrap_err();
            assert!(
                matches!(err, Error::CommitMetaInvalid(ref got) if got == rejected),
                "rejected {rejected}: {err:?}"
            );
        }

        let err = parse_meta_flag(Some("{"), UserMeta::Keep).unwrap_err();
        assert!(matches!(err, Error::Json(_)), "bad JSON: {err:?}");
    }

    /// The workflow flags are first-push-only: without `--bucket` there is no
    /// remote to set, so `--workflow` is rejected at the dispatch boundary.
    #[test(tokio::test)]
    async fn test_push_workflow_without_bucket_errors() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let push_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: None,
                host: None,
                origin: None,
                workflow: Some("x".to_string()),
                no_workflow: false,
            },
        };

        assert!(matches!(
            init(push_args).await,
            Err(Error::WorkflowRequiresBucket)
        ));

        Ok(())
    }

    /// `--no-workflow` is likewise first-push-only and rejected without `--bucket`.
    #[test(tokio::test)]
    async fn test_push_no_workflow_without_bucket_errors() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let push_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: None,
                host: None,
                origin: None,
                workflow: None,
                no_workflow: true,
            },
        };

        assert!(matches!(
            init(push_args).await,
            Err(Error::WorkflowRequiresBucket)
        ));

        Ok(())
    }

    /// With `--bucket`/`--host` present the workflow flag is accepted: the
    /// boundary check passes and the command threads the intent into
    /// `push::Input`, reaching `push_package` (which then reports the missing
    /// local package rather than a workflow error).
    #[test(tokio::test)]
    async fn test_push_workflow_with_bucket_is_accepted() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let push_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: Some("some-bucket".to_string()),
                host: Some(fixtures::host()),
                origin: None,
                workflow: Some("x".to_string()),
                no_workflow: false,
            },
        };

        let mut output = Vec::new();
        let result = init(push_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Package foo/bar not found\n");

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_pull_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::outdated as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        // No file group: `install` ran with `paths: None`, so this copy tracks
        // nothing and every path the revision changed falls outside the touch
        // set — nothing moved on disk, and claiming otherwise would be false.
        // The revision's own message is then the only thing the pull can report,
        // which is exactly why it is worth reporting: without it the line says
        // no more than it did before.
        assert_eq!(
            output_str,
            format!(
                "Revision \"{}\" pulled\nLatest revision: Today's Date: 2024-07-29 11:53:53\n",
                pkg::LATEST_TOP_HASH
            )
        );

        Ok(())
    }

    /// A second pull finds nothing new and still succeeds: text says so, and
    /// the JSON keeps the pull shape with no `hash`.
    #[test(tokio::test)]
    async fn live_pull_twice_succeeds() -> Result<(), Error> {
        use crate::cli::fixtures::packages::outdated as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;
        let pull_args = |json| Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        let mut stderr = Vec::new();
        let result = init(pull_args(false)).await?;
        print(result, Format::Text, &mut Vec::new(), &mut stderr)?;
        assert_eq!(String::from_utf8_lossy(&stderr), "");

        let mut output = Vec::new();
        let result = init(pull_args(false)).await?;
        print(result, Format::Text, &mut output, &mut stderr)?;
        assert_eq!(String::from_utf8(output).unwrap(), "Already up to date\n");
        assert_eq!(String::from_utf8_lossy(&stderr), "");

        let mut output = Vec::new();
        let result = init(pull_args(true)).await?;
        print(result, Format::Json, &mut output, &mut stderr)?;
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["hash"], serde_json::Value::Null);
        assert_eq!(String::from_utf8_lossy(&stderr), "");

        Ok(())
    }

    /// The report, end to end against a real package. This is the only test
    /// that proves the whole path — engine grouping, the CLI's wording, and a
    /// manifest pair that actually differs — rather than a hand-built delta.
    ///
    /// It is also the span case: `latest` is r3, so installing r1 and pulling
    /// crosses two revisions in one operation, and only r3's message is in hand.
    /// There is no parent pointer to walk, so nothing can report r2's.
    ///
    /// The silences are the load-bearing half. `install` takes no paths, so this
    /// copy tracks nothing: `modify.txt` and `keep.txt` were modified and
    /// `remove.txt` removed, and none may appear — nothing moved on disk, and
    /// naming them would report writes that did not happen.
    #[test(tokio::test)]
    async fn live_pull_reports_what_the_revision_brought() -> Result<(), Error> {
        use crate::cli::fixtures::packages::revision_report as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::R1_URI).await?;

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();

        assert_eq!(
            output_str,
            format!(
                concat!(
                    "Revision \"{}\" pulled\n",
                    "7 files new, not downloaded:\n",
                    "  add/deeply/nested/directory/with/a/very-long-name/summary-of-everything.parquet\n",
                    "  add/five.txt\n",
                    "  add/four.txt\n",
                    "  add/one.txt\n",
                    "  add/six.txt\n",
                    "  add/three.txt\n",
                    "  add/two.txt\n",
                    "Latest revision: {}\n",
                ),
                pkg::R3_TOP_HASH,
                pkg::R3_MESSAGE,
            )
        );

        // Stated as its own assertion rather than left implicit in the string
        // above: these three changed on the remote and must appear in no group,
        // because this copy does not track them.
        //
        // Checked against the group lines only, not the whole output: the
        // author's message is prose and legitimately names files — r3's says
        // "modifies keep.txt" — so a substring search over everything would
        // fail on the message rather than on a group, which is what happened
        // when this was written the naive way.
        let groups = output_str
            .split("Latest revision:")
            .next()
            .expect("split always yields one part");
        for silent in ["modify.txt", "keep.txt", "remove.txt"] {
            assert!(
                !groups.contains(silent),
                "{silent} changed on the remote but nothing moved here, so no group may name it"
            );
        }
        Ok(())
    }

    /// The same fixture with its files actually checked out, which is what makes
    /// three of the four groups reachable: a tracked path the remote modified is
    /// rewritten (`updated`), a tracked path it dropped is deleted (`removed`),
    /// and its additions stay listed under the CLI's sparse scope.
    ///
    /// The pair with the test above is the point. Same revisions, same remote
    /// changes, and the report differs entirely — because what a pull *reports*
    /// follows what this copy tracks, not what the remote did. That is the claim
    /// the grouping rests on, and no unit test can make it against real
    /// manifests.
    ///
    /// The fourth group, `added` proper, is unreachable here by design: it needs
    /// whole-package scope, and the CLI always asks for the narrow one so that
    /// state a desktop wrote cannot change what a script does.
    #[test(tokio::test)]
    async fn live_pull_reports_updates_and_removals_for_tracked_paths() -> Result<(), Error> {
        use crate::cli::fixtures::packages::revision_report as pkg;
        use crate::cli::model::install_paths_into_temp_dir;

        let tracked = ["keep.txt", "modify.txt", "remove.txt"]
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        let (_, _, temp_dir) = install_paths_into_temp_dir(pkg::R1_URI, Some(tracked)).await?;

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();

        assert_eq!(
            output_str,
            format!(
                concat!(
                    "Revision \"{}\" pulled\n",
                    "7 files new, not downloaded:\n",
                    "  add/deeply/nested/directory/with/a/very-long-name/summary-of-everything.parquet\n",
                    "  add/five.txt\n",
                    "  add/four.txt\n",
                    "  add/one.txt\n",
                    "  add/six.txt\n",
                    "  add/three.txt\n",
                    "  add/two.txt\n",
                    "2 files updated:\n",
                    "  keep.txt\n",
                    "  modify.txt\n",
                    "1 file removed:\n",
                    "  remove.txt\n",
                    "Latest revision: {}\n",
                ),
                pkg::R3_TOP_HASH,
                pkg::R3_MESSAGE,
            )
        );
        Ok(())
    }

    /// The mapped reader half of [`live_pull_leaves_a_memory_mapped_file_readable`]:
    /// map the file, say so, wait to be told, then read the tail and say what it
    /// found. Never exits on its own — the parent decides when it is finished,
    /// so the read happens strictly after the pull has landed.
    #[cfg(unix)]
    fn mapped_reader_child(path: &str, len: usize) {
        use std::os::unix::io::AsRawFd as _;

        let file = std::fs::File::open(path).expect("map target");
        // SAFETY: a private read-only mapping of a file this process holds open;
        // the pointer is used only for the single read below.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        assert!(!std::ptr::eq(addr, libc::MAP_FAILED), "mmap failed");
        println!("MAPPED");
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line).ok();
        // The tail — the part a shrinking replacement leaves past end-of-file.
        // SAFETY: within the mapping established above.
        let byte = unsafe { std::ptr::read_volatile((addr as *const u8).add(len - 1)) };
        println!("READ {byte}");
    }

    /// A pull must not disturb a process that has a package file mapped.
    ///
    /// Replacing a working file by copying onto it truncates the destination
    /// before refilling it, and a mapping of a truncated file faults: a read
    /// past the new end of file raises `SIGBUS` and kills the reader outright.
    /// Mapping package files is ordinary for what this tool carries — HDF5,
    /// Zarr, Arrow, `numpy` `mmap_mode` all do it — so under the old write a
    /// pull arriving mid-analysis could take the analysis down with it. Writing
    /// by rename leaves the mapping bound to the inode it opened, which stays
    /// readable until the mapping is dropped.
    ///
    /// This is the one claim in the change whose mechanism is entirely the
    /// kernel's, so a mock filesystem could not test it at all.
    ///
    /// The fixture shrinks 4 MiB to 64 KiB deliberately. Were the replacement
    /// the same size, a copy would only fault during its own truncate window and
    /// the test would race; a file that ends up shorter leaves the mapped tail
    /// permanently past end-of-file, so the old behaviour fails every time.
    ///
    /// Unix only: `SIGBUS` on a truncated mapping and a mapping that outlives
    /// a rename over its file are POSIX semantics. Windows refuses to truncate
    /// or replace a mapped file at all, so this failure cannot happen there.
    #[cfg(unix)]
    #[test(tokio::test)]
    #[allow(
        clippy::zombie_processes,
        reason = "the child is killed and reaped on every path out, including a failed pull and each failed assertion; the analysis cannot follow it through the branches"
    )]
    async fn live_pull_leaves_a_memory_mapped_file_readable() -> Result<(), Error> {
        use crate::cli::fixtures::packages::shrinking as pkg;
        use crate::cli::model::install_paths_into_temp_dir;

        const CHILD_ENV: &str = "QUILT_MMAP_TEST_FILE";

        // The child half lives in `mapped_reader_child`.
        if let Ok(path) = std::env::var(CHILD_ENV) {
            mapped_reader_child(&path, pkg::R1_LEN);
            return Ok(());
        }

        let tracked = vec![std::path::PathBuf::from(pkg::MAPPED)];
        let (_, _, temp_dir) = install_paths_into_temp_dir(pkg::R1_URI, Some(tracked)).await?;
        let root = temp_dir.path().to_path_buf();
        let mapped = root.join(pkg::NAMESPACE_STR).join(pkg::MAPPED);
        assert_eq!(
            std::fs::metadata(&mapped).unwrap().len(),
            pkg::R1_LEN as u64,
            "r1 should be installed at its full size"
        );

        let mut kid = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "cli::tests::live_pull_leaves_a_memory_mapped_file_readable",
                "--exact",
                "--nocapture",
            ])
            .env(CHILD_ENV, &mapped)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn the reader");

        // Only pull once the mapping exists, or the test proves nothing. The
        // child is a test binary, so its stdout carries libtest's own chatter
        // before ours — scan for the marker rather than assuming it comes first.
        fn wait_for(out: &mut impl std::io::BufRead, marker: &str) -> Option<String> {
            let mut line = String::new();
            while {
                line.clear();
                std::io::BufRead::read_line(out, &mut line).unwrap_or(0) > 0
            } {
                if line.starts_with(marker) {
                    return Some(line.trim_end().to_string());
                }
            }
            None
        }

        // Everything from here reaps the child before asserting: a panic
        // between the spawn and the wait would leave it holding its mapping.
        let mut out = std::io::BufReader::new(kid.stdout.take().unwrap());
        let mapped_ok = wait_for(&mut out, "MAPPED").is_some();
        if !mapped_ok {
            kid.kill().ok();
            kid.wait().ok();
            panic!("the child never reported a mapping");
        }

        let pull = Args {
            domain: Some(root.clone()),
            home: Some(root.clone()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };
        let mut output = Vec::new();
        let pulled = async {
            let result = init(pull).await?;
            print(result, Format::Text, &mut output, &mut Vec::new())?;
            Ok::<_, Error>(())
        }
        .await;
        if pulled.is_err() {
            kid.kill().ok();
            kid.wait().ok();
        }
        pulled?;
        let report = String::from_utf8(output).unwrap();
        let landed_r2 = report.contains(pkg::R2_TOP_HASH);
        let shrank = std::fs::metadata(&mapped).unwrap().len() < pkg::R1_LEN as u64;
        if !(landed_r2 && shrank) {
            kid.kill().ok();
            kid.wait().ok();
            assert!(landed_r2, "the pull should land r2, got: {report}");
            assert!(shrank, "r2 should be the shorter file");
        }

        // Now let the reader touch its mapping. Under a copy-based write this
        // is where it dies on SIGBUS.
        use std::io::Write as _;
        kid.stdin.take().unwrap().write_all(b"\n").ok();
        let read = wait_for(&mut out, "READ");
        let status = kid.wait().expect("reader should be reapable");

        assert!(
            read.is_some(),
            "the mapped reader never completed its read — the pull killed it: {status:?}"
        );
        assert!(
            status.success(),
            "the mapped reader did not exit cleanly: {status:?}"
        );
        Ok(())
    }

    /// A real `SIGKILL` in the middle of a real pull.
    ///
    /// Every other interruption in this workspace is an error return — a fetch
    /// that fails, a rename that cannot land. Those model the *disk* state
    /// correctly, because what makes a file whole is `rename` being atomic in
    /// the kernel, which holds however the process ends. What they cannot model
    /// is a process that stops between two syscalls with nothing unwinding: no
    /// `?` propagating, no cleanup running. The invariant claims to survive
    /// exactly that.
    ///
    /// Uses `reference/large` rather than the reporting fixture because this is
    /// the one test whose subject is *timing*: a pull of a few small text files
    /// finishes before anything can interrupt it, and a kill landing after the
    /// pull completed would pass while proving nothing.
    ///
    /// The kill is gated on a **staging file existing**, not on elapsed time.
    /// That is what makes the assertion strong rather than merely safe: staging
    /// completes for the whole touch set before the first rename, so a staging
    /// file proves the apply is under way *and* has not begun swapping — which
    /// licenses asserting every path is still at r1, not the weaker "r1 or r2".
    #[test(tokio::test)]
    async fn live_killed_pull_leaves_every_tracked_path_at_the_old_revision() -> Result<(), Error> {
        use crate::cli::fixtures::packages::large as pkg;
        use crate::cli::model::install_paths_into_temp_dir;

        const CHILD_ENV: &str = "QUILT_KILLED_PULL_DOMAIN";

        fn pull_args(root: &std::path::Path) -> Args {
            Args {
                domain: Some(root.to_path_buf()),
                home: Some(root.to_path_buf()),
                verbose: false,
                log: None,
                format: None,
                json: false,
                command: Commands::Pull {
                    pkg: PackageRef {
                        namespace: Some(pkg::NAMESPACE_STR.to_string()),
                    },
                },
            }
        }

        // The child: pull, and expect to die inside it.
        if let Ok(root) = std::env::var(CHILD_ENV) {
            let _ = init(pull_args(std::path::Path::new(&root))).await;
            return Ok(());
        }

        let tracked = pkg::PATHS
            .iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        let (_, _, temp_dir) = install_paths_into_temp_dir(pkg::R1_URI, Some(tracked)).await?;
        let root = temp_dir.path().to_path_buf();
        let working = |name: &str| root.join(pkg::NAMESPACE_STR).join(name);

        let at_r1: Vec<Vec<u8>> = pkg::PATHS
            .iter()
            .map(|name| std::fs::read(working(name)).expect("installed at r1"))
            .collect();

        let mut kid = std::process::Command::new(std::env::current_exe().unwrap())
            // The full path: libtest's `--exact` matches the whole name, and a
            // filter matching nothing runs nothing and exits happily.
            .args([
                "cli::tests::live_killed_pull_leaves_every_tracked_path_at_the_old_revision",
                "--exact",
                "--nocapture",
            ])
            .env(CHILD_ENV, &root)
            .spawn()
            .expect("spawn the pull that gets killed");

        // Poll rather than sleep: a fixed delay on a slow machine kills before
        // anything has happened, and on fixed code a pull that never started
        // looks exactly like one that was interrupted safely.
        let staging = root.join(".quilt").join("staging");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut staged = false;
        while std::time::Instant::now() < deadline {
            // Wait for *every* path to be staged, not merely one. That state
            // only exists because staging completes for the whole touch set
            // before the first rename: a shape that staged and swapped each
            // file in turn could never hold two at once, so this both proves
            // the swap has not begun and fails against that shape rather than
            // racing it.
            if std::fs::read_dir(&staging).is_ok_and(|entries| {
                entries.flatten().any(|run| {
                    std::fs::read_dir(run.path())
                        .is_ok_and(|f| f.flatten().count() >= pkg::PATHS.len())
                })
            }) {
                staged = true;
                break;
            }
            if kid.try_wait().ok().flatten().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        kid.kill().ok();
        kid.wait().ok();
        assert!(
            staged,
            "the whole touch set was never staged at once, so either the pull was not interrupted mid-apply or files are being swapped in one at a time"
        );

        // Killed before the swap, so nothing was written into the working tree:
        // every path is still whole, and still at r1.
        for (name, before) in pkg::PATHS.iter().zip(&at_r1) {
            let after = std::fs::read(working(name))
                .unwrap_or_else(|err| panic!("{name} is absent after a killed pull: {err:?}"));
            assert_eq!(
                after.len(),
                before.len(),
                "{name} changed size, so it holds neither revision whole"
            );
            assert!(&after == before, "{name} was written before the swap began");
        }

        // And the tree is an ordinary retry: pulling again completes, and lands
        // exactly r2 rather than merely changing something.
        let mut output = Vec::new();
        let result = init(pull_args(&root)).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let report = String::from_utf8(output).unwrap();
        assert!(
            report.contains(pkg::R2_TOP_HASH),
            "the retry should land r2, got: {report}"
        );
        for (name, before) in pkg::PATHS.iter().zip(&at_r1) {
            let after = std::fs::read(working(name)).expect("present after the retry");
            assert!(&after != before, "{name} still holds r1 after the retry");
        }
        Ok(())
    }

    /// Local work the remote did not touch survives the pull, and appears in no
    /// group — it is the user's change, not news from the remote.
    ///
    /// Installed from **r2** rather than r1 for a reason worth keeping: every
    /// path r1 holds is touched by r3, so from r1 there is nothing for a local
    /// edit to survive on without colliding. `add/one.txt` arrives in r2 and r3
    /// leaves it alone.
    #[test(tokio::test)]
    async fn live_pull_keeps_local_work_the_remote_did_not_touch() -> Result<(), Error> {
        use crate::cli::fixtures::packages::revision_report as pkg;
        use crate::cli::model::install_paths_into_temp_dir;

        let tracked = ["add/one.txt", "keep.txt"]
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        let (_, _, temp_dir) = install_paths_into_temp_dir(pkg::R2_URI, Some(tracked)).await?;

        let edited = temp_dir.path().join(pkg::NAMESPACE_STR).join("add/one.txt");
        std::fs::write(&edited, "MY LOCAL EDIT\n").expect("the path was just checked out");

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();

        assert_eq!(
            output_str,
            format!(
                concat!(
                    "Revision \"{}\" pulled\n",
                    "1 file new, not downloaded:\n",
                    "  add/six.txt\n",
                    "1 file updated:\n",
                    "  keep.txt\n",
                    "Latest revision: {}\n",
                ),
                pkg::R3_TOP_HASH,
                pkg::R3_MESSAGE,
            )
        );

        // The half a report cannot show: the edit is still there. A pull that
        // named nothing and quietly overwrote it would pass the assertion above.
        assert_eq!(
            std::fs::read_to_string(&edited).unwrap(),
            "MY LOCAL EDIT\n",
            "the pull overwrote local work the remote had not touched"
        );
        Ok(())
    }

    /// A path added on both sides with differing content blocks the whole pull,
    /// so there is no partial arrival and no report at all.
    ///
    /// The reconcile is atomic: `base` advances only when every path applies. A
    /// report here would describe files that did not move.
    #[test(tokio::test)]
    async fn live_pull_blocked_by_a_conflict_reports_nothing() -> Result<(), Error> {
        use crate::cli::fixtures::packages::revision_report as pkg;
        use crate::cli::model::install_paths_into_temp_dir;

        let tracked = vec![std::path::PathBuf::from("keep.txt")];
        let (_, _, temp_dir) = install_paths_into_temp_dir(pkg::R1_URI, Some(tracked)).await?;

        // Same logical key r2 adds, different bytes: the both-added arm.
        let root = temp_dir.path().join(pkg::NAMESPACE_STR).join("add");
        std::fs::create_dir_all(&root).expect("the package root exists");
        std::fs::write(root.join("one.txt"), "DIFFERENT CONTENT THAN THE REMOTE\n")
            .expect("just created the directory");

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
            },
        };

        // The refusal travels as a `Std::Err`, not as an `Err` — the CLI's
        // output contract sends it to stderr and exits non-zero rather than
        // propagating. So the assertion is about what the user sees.
        let mut out = Vec::new();
        let mut errs = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut out, &mut errs).ok();
        let out = String::from_utf8(out).unwrap();
        let errs = String::from_utf8(errs).unwrap();

        assert!(
            errs.contains("add/one.txt"),
            "the refusal must name the conflicting path: {errs}"
        );
        // The claim worth testing: nothing was applied, so nothing is reported.
        // A report here would describe files that did not move.
        assert!(
            out.is_empty(),
            "a blocked pull applies nothing and must report nothing, got: {out}"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_pull_invalid() -> Result<(), Error> {
        // Create temporary directory for domain
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Pull {
                pkg: PackageRef {
                    namespace: Some("in/valid".to_string()),
                },
            },
        };

        // Test init with invalid namespace
        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Package in/valid not found\n");

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_uninstall_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let uninstall_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Uninstall {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
                prune: false,
                package: None,
                dataspace: None,
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(uninstall_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            format!("Package {} successfully uninstalled\n", pkg::NAMESPACE_STR)
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_uninstall_invalid() -> Result<(), Error> {
        // Create temporary directory for domain
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let uninstall_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Uninstall {
                pkg: PackageRef {
                    namespace: Some("in/valid".to_string()),
                },
                prune: false,
                package: None,
                dataspace: None,
            },
        };

        // Test init with invalid namespace
        let mut output = Vec::new();
        let result = init(uninstall_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.ends_with("The given package is not installed: in/valid\n"));

        Ok(())
    }

    /// Unix only: it denies access with POSIX mode bits, and Windows ignores
    /// the read-only attribute on directories.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn test_list_invalid() -> Result<(), Error> {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::Builder;

        // Create write-only temporary directory to trigger permission error
        let write_only = Permissions::from_mode(0o200);
        let temp_dir = Builder::new().permissions(write_only).tempdir()?;

        let list_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::List {
                fetch: false,
                dataspace: None,
            },
        };

        // Default home initialization now reaches the same write-protected
        // lineage before the list command can render a command-level error.
        let err = init(list_args).await.unwrap_err();
        assert!(err.to_string().contains("Permission denied"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_list_valid() -> Result<(), Error> {
        // Create temporary directory for domain
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let list_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::List {
                fetch: false,
                dataspace: None,
            },
        };

        // Test init with empty domain
        let mut output = Vec::new();
        let result = init(list_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "No installed packages\n");

        Ok(())
    }

    /// The only place the whole `--json` chain is joined: a real command's
    /// output travelling through `init` and `print` in JSON mode. Every other
    /// JSON test calls `to_json` on a hand-built `Output`, so the flag, the
    /// `Format` it selects, and the printer that reads it are each proven
    /// separately and never together.
    ///
    /// Covers the commands reachable without a remote; `browse`, `install`,
    /// `login`, `pull`, `push` and `role` all need one.
    #[test(tokio::test)]
    #[allow(clippy::too_many_lines, reason = "one table of every local command")]
    async fn json_mode_emits_one_parseable_object_per_local_command() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;
        let dir = temp_dir.path().to_path_buf();

        let args = |command| Args {
            domain: Some(dir.clone()),
            home: Some(dir.clone()),
            verbose: false,
            log: None,
            format: None,
            json: true,
            command,
        };
        let pkg = || PackageRef {
            namespace: Some("test/pkg".to_string()),
        };

        let source = dir.join("source");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("data.csv"), "a,b\n1,2")?;

        let commands = vec![
            (
                "create",
                Commands::Create {
                    namespace: Some("test/pkg".to_string()),
                    package: None,
                    dataspace: None,
                    access: None,
                    source: Some(source.clone()),
                    message: Some("first".to_string()),
                    user_meta: None,
                },
            ),
            (
                "list",
                Commands::List {
                    fetch: false,
                    dataspace: None,
                },
            ),
            (
                "status",
                Commands::Status {
                    pkg: pkg(),
                    ignored: false,
                    dataspace: None,
                },
            ),
            (
                "commit",
                Commands::Commit {
                    message: "second".to_string(),
                    user_meta: None,
                    pkg: pkg(),
                    package: None,
                    dataspace: None,
                    workflow: None,
                    no_workflow: true,
                },
            ),
            (
                "log",
                Commands::Log {
                    pkg: pkg(),
                    package: None,
                    dataspace: None,
                },
            ),
            (
                "undo-commit",
                Commands::UndoCommit {
                    pkg: pkg(),
                    package: None,
                    dataspace: None,
                },
            ),
            (
                "remove-revisions",
                Commands::RemoveRevisions {
                    pkg: pkg(),
                    count: None,
                    package: None,
                    dataspace: None,
                    revision: Vec::new(),
                },
            ),
            (
                "uninstall",
                Commands::Uninstall {
                    pkg: pkg(),
                    prune: false,
                    package: None,
                    dataspace: None,
                },
            ),
        ];

        for (name, command) in commands {
            // `commit` refuses an unchanged tree, so give it one edit to find.
            if name == "commit" {
                std::fs::write(dir.join("test/pkg/data.csv"), "a,b\n3,4")?;
            }

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();

            let result = init(args(command)).await?;
            print(result, Format::Json, &mut stdout, &mut stderr)?;

            let parsed: serde_json::Value = serde_json::from_slice(&stdout).unwrap_or_else(|err| {
                panic!(
                    "`{name} --json` emitted unparseable stdout ({err}): {}",
                    String::from_utf8_lossy(&stdout)
                )
            });
            assert!(
                parsed.is_object(),
                "`{name} --json` emitted {parsed}, not a bare object"
            );
            assert!(
                stderr.is_empty(),
                "`{name} --json` wrote to stderr: {}",
                String::from_utf8_lossy(&stderr)
            );
        }

        Ok(())
    }

    /// The failure half of the same chain, end to end rather than against a
    /// stand-in: stdout carries nothing a consumer could half-parse, and the
    /// error object on stderr has a kind to branch on.
    #[test(tokio::test)]
    async fn json_mode_failure_leaves_stdout_empty_and_stderr_parseable() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;
        let args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: true,
            command: Commands::Status {
                pkg: PackageRef {
                    namespace: Some("no/such".to_string()),
                },
                ignored: false,
                dataspace: None,
            },
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        print(init(args).await?, Format::Json, &mut stdout, &mut stderr)?;

        assert!(stdout.is_empty(), "stdout must stay empty on failure");
        let parsed: serde_json::Value =
            serde_json::from_slice(&stderr).expect("stderr carries parseable JSON");
        assert_eq!(parsed["error"]["kind"], "namespace_not_found");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_install_invalid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::invalid as pkg;

        // Create temporary directory for domain
        let temp_dir = tempfile::tempdir()?;
        let domain = Some(temp_dir.path().to_path_buf());
        let home = domain.clone();

        let install_args = Args {
            domain,
            home,
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Install {
                namespace: None,
                uri: pkg::URI.to_string(),
                path: None,
                keep_all: false,
                keep_folder: Vec::new(),
                no_download: false,
                dataspace: None,
                access: None,
                folder: None,
            },
        };

        // Test init with invalid URI
        let mut output = Vec::new();
        let result = init(install_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            format!(
                "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: {}\n",
                pkg::URI
            )
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_browse_valid() -> Result<(), Error> {
        use crate::cli::fixtures::get_browse_output;
        use crate::cli::fixtures::packages::default as pkg;

        // Create temporary directory for domain
        let temp_dir = tempfile::tempdir()?;
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);

        let browse_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Browse { uri },
        };

        // Test init with valid URI
        let mut output = Vec::new();
        let result = init(browse_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, format!("{}\n", get_browse_output()?));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_browse_invalid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::invalid as pkg;

        // Create temporary directory for domain
        let temp_dir = tempfile::tempdir()?;

        let browse_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Browse {
                uri: pkg::URI.to_string(),
            },
        };

        // Test init with invalid URI
        let mut output = Vec::new();
        let result = init(browse_args).await?;
        print(result, Format::Text, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            format!(
                "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: {}\n",
                pkg::URI
            )
        );

        Ok(())
    }
}

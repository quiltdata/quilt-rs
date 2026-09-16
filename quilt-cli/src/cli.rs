//! Not a part of the library and meant to be an independent project.
//! This is a CLI frontend for `quilt_rs`.

use std::path::Path;
use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use tracing::log;

use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::Host;
use quilt_uri::Namespace;

mod browse;
mod commit;
mod create;
mod history;
mod install;
mod list;
mod login;
mod model;
mod output;
mod pull;
mod push;
mod role;
mod status;
mod text;
mod undo_commit;
mod uninstall;

#[cfg(test)]
mod fixtures;

use model::Model;
pub use output::Format;
pub use output::Std;
pub use output::print;

const DOMAIN_DIR_NAMESPACE: &str = "com.quiltdata.quilt-sync";

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

fn parse_optional_namespace(namespace: Option<String>) -> Result<Option<Namespace>, Error> {
    Ok(match namespace {
        Some(namespace) => Some(namespace.try_into()?),
        None => None,
    })
}

fn get_domain_dir(dir_arg: Option<PathBuf>) -> Result<PathBuf, Error> {
    match dir_arg {
        Some(user_specified_dir) => Ok(user_specified_dir),
        None => match dirs::data_local_dir() {
            Some(default_user_dir) => Ok(default_user_dir.join(DOMAIN_DIR_NAMESPACE)),
            None => Err(Error::Domain),
        },
    }
}

fn get_default_home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir()
        .map(|user_home| user_home.join(quilt_rs::DEFAULT_HOME_DIR_NAME))
        .ok_or(Error::Home)
}

async fn initialize_home(model: &Model, home: Option<PathBuf>) -> Result<(), Error> {
    if let Some(dir) = home {
        model.set_home(dir).await?;
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

    /// Absolute path for the directory, where all packages will store their mutable files.
    /// Defaults to `~/QuiltSync` on first use. Ex. /home/user/QuiltSync
    #[arg(long)]
    home: Option<PathBuf>,

    /// Path to local domain
    #[arg(short, long)]
    domain: Option<PathBuf>,

    /// Enable INFO-level logging; use `RUST_LOG` for finer-grained filtering.
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    /// Print machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub(crate) json: bool,
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
enum Commands {
    /// Browse remote manifest
    Browse {
        #[arg(value_name = "PKG_URI")]
        uri: String,
    },
    /// Create a new local package
    Create {
        /// Namespace for the package, e.g. foo/bar
        #[arg(short, long)]
        namespace: String,
        /// Optional source directory to populate the package from
        #[arg(short, long)]
        source: Option<PathBuf>,
        /// Commit message for the initial revision
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Commit new package revision
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
        /// JSON string for user meta
        #[arg(short, long)]
        user_meta: Option<String>,
        #[command(flatten)]
        pkg: PackageRef,
        /// Workflow ID
        /// Ex. `"my_workflow"`
        /// Omit to use the bucket's default workflow.
        #[arg(short, long)]
        workflow: Option<String>,
        /// Commit with no workflow (explicit opt-out)
        #[arg(long, conflicts_with = "workflow")]
        no_workflow: bool,
    },
    /// Install package locally
    Install {
        /// Source URI for the package.
        /// Ex. `quilt+s3://bucket#package=foo/bar`
        #[arg(value_name = "PKG_URI")]
        uri: String,
        /// Namespace for the package, ex. foo/bar.
        #[arg(short, long)]
        namespace: Option<String>,
        /// Logical key relative to the root of the package to be installed locally.
        /// You can provide multiple paths.
        #[arg(short, long)]
        path: Option<Vec<PathBuf>>,
    },
    /// Authenticate against a Quilt stack
    Login {
        /// Code from the `https://QUILT_STACK/code` page
        #[arg(short, long)]
        code: Option<String>,
        #[arg(long)]
        host: Host,
    },
    /// List installed packages
    List,
    /// List the revisions of a package this copy has, newest first.
    ///
    /// Ordered by when this copy obtained each revision, which is all that is
    /// recorded locally — a manifest carries no timestamp of its own, so for a
    /// revision fetched from a remote this is the fetch time, not the commit
    /// time.
    Log {
        #[command(flatten)]
        pkg: PackageRef,
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
        #[arg(short, long, requires = "origin")]
        bucket: Option<String>,
        /// Remote host (required for first push of local-only packages)
        /// Ex. open.quiltdata.com
        #[arg(short, long, requires = "bucket")]
        origin: Option<Host>,
        /// Workflow ID for the first push (requires --bucket/--origin).
        /// Ex. `"my_workflow"`
        /// Omit to use the bucket's default workflow.
        /// Meaningful only on a first push: a subsequent push uploads an
        /// already-created commit whose workflow was chosen at commit time.
        #[arg(short, long)]
        workflow: Option<String>,
        /// First push with no workflow (explicit opt-out; requires --bucket/--origin)
        #[arg(long, conflicts_with = "workflow")]
        no_workflow: bool,
    },
    /// Show or switch your active role on a Quilt stack
    ///
    /// The active role is server-side and global: it decides what every Quilt
    /// client signed in as you can read and write, this CLI and the desktop
    /// app alike. Listing marks the active role with a leading asterisk.
    Role {
        /// Quilt stack to read the role from.
        /// Ex. open.quiltdata.com
        #[arg(long)]
        host: Host,
        /// Switch to this role instead of listing the roles you hold
        #[arg(long, value_name = "ROLE")]
        set: Option<String>,
    },
    /// Status of the package: modified, up-to-date, outdated
    Status {
        #[command(flatten)]
        pkg: PackageRef,
    },
    /// Undo the newest commit, restoring the revision before it
    ///
    /// Available while the package's commit chain still reaches back, which in
    /// practice means before its first push. Refuses if any tracked file has
    /// uncommitted changes.
    UndoCommit {
        #[command(flatten)]
        pkg: PackageRef,
    },
    /// Uninstall package from local domain
    Uninstall {
        #[command(flatten)]
        pkg: PackageRef,
    },
}

#[allow(
    clippy::too_many_lines,
    reason = "cohesive top-level CLI command dispatch"
)]
pub async fn init(args: Args) -> Result<Std, Error> {
    // NOTE: every command should have some domain,
    //       because domain stores credentials
    //       It's optional for user, but we use one anyway.
    //       If it is None, we use:
    //         * home directory ~/.local/share/com.quiltdata.quilt-sync`
    //         * or temporary directory
    let root_dir = get_domain_dir(args.domain)?;
    let m = Model::from(root_dir);

    // Preserve an existing home, honor an explicit --home override, and set
    // the default for a new domain on first use.
    initialize_home(&m, args.home).await?;

    match args.command {
        Commands::Browse { uri } => {
            let args = browse::Input { uri };

            log::debug!("Browsing {args:?}");
            Ok(browse::command(m, args).await)
        }
        Commands::Create {
            namespace,
            source,
            message,
        } => {
            let args = create::Input {
                namespace: namespace.try_into()?,
                source,
                message,
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
        } => {
            let namespace = pkg.resolve(&m).await?;
            let user_meta = match &user_meta {
                Some(object) => match serde_json::from_str(object)? {
                    serde_json::Value::Object(object) => {
                        UserMeta::Set(serde_json::Value::Object(object))
                    }
                    _ => {
                        return Err(Error::CommitMetaInvalid(object.clone()));
                    }
                },
                None => UserMeta::Keep,
            };
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
        Commands::Install {
            namespace,
            path,
            uri,
        } => {
            let args = install::Input {
                namespace: parse_optional_namespace(namespace)?,
                paths: path,
                uri,
            };

            log::debug!("Installing {args:?}");
            Ok(install::command(m, args).await)
        }
        Commands::Login { code, host } => {
            if let Some(code) = code {
                let args = login::Input { code, host };

                log::debug!("Logging in {args:?}");
                Ok(login::command(m, args).await)
            } else {
                // TODO: Check the lineage, if there are some `package.remote.catalog`
                Ok(Std::Err(Error::LoginRequired(host)))
            }
        }
        Commands::List => {
            log::info!("Listing installed packages");
            Ok(list::command(m).await)
        }
        Commands::Log { pkg } => {
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
            origin,
            workflow,
            no_workflow,
        } => {
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
        Commands::Role { host, set } => {
            let args = role::Input { host, set };

            log::debug!("Role {args:?}");
            Ok(role::command(m, args).await)
        }
        Commands::Status { pkg } => {
            let namespace = pkg.resolve(&m).await?;
            let args = status::Input {
                namespace,
                host_config: None,
            };

            log::debug!("Status {args:?}");
            Ok(status::command(m, args).await)
        }
        Commands::UndoCommit { pkg } => {
            let namespace = pkg.resolve(&m).await?;
            let args = undo_commit::Input { namespace };

            log::debug!("Undoing commit {args:?}");
            Ok(undo_commit::command(m, args).await)
        }
        Commands::Uninstall { pkg } => {
            let namespace = pkg.resolve(&m).await?;
            let args = uninstall::Input { namespace };

            log::debug!("Uninstalling {args:?}");
            Ok(uninstall::command(m, args).await)
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Domain directory is required. We store files and credentials there")]
    Domain,

    #[error("Could not determine the home directory. Pass --home to specify one")]
    Home,

    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),

    #[error(
        r"
Please visit https://{0}/code to get your code.
Then run:
> quilt_rs login --host {0} --code YOUR_CODE"
    )]
    LoginRequired(Host),

    #[error("Package {0} not found")]
    NamespaceNotFound(Namespace),

    #[error(
        "Could not infer a namespace from the current directory. Pass --namespace or run inside <home>/<prefix>/<name>"
    )]
    NamespaceRequired,

    #[error("Invalid JSON for user_meta object. Object is required")]
    CommitMetaInvalid(String),

    #[error("workflow id cannot be empty; omit --workflow to use the bucket's default workflow")]
    WorkflowEmpty,

    #[error(
        "--workflow/--no-workflow apply only to a first push; pass --bucket and --origin to set the remote, or omit them (the workflow was chosen at commit time)"
    )]
    WorkflowRequiresBucket,

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

impl From<quilt_uri::UriError> for Error {
    fn from(err: quilt_uri::UriError) -> Error {
        Error::Quilt(quilt_rs::Error::Uri(err))
    }
}

impl Error {
    /// A stable machine-readable discriminant for `--json` consumers, so a
    /// caller can branch without matching English prose.
    ///
    /// Reaches one level into `quilt_rs::Error` for the variants a caller can
    /// act on. Everything else is `quilt_error`: a new library variant lands in
    /// the catch-all rather than breaking the build, and earns its own kind only
    /// when something needs to branch on it.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Domain => "domain",
            Error::Home => "home",
            Error::Quilt(err) => match err {
                quilt_rs::Error::Uri(_) => "invalid_uri",
                quilt_rs::Error::Auth(..) => "auth",
                quilt_rs::Error::Login(_) => "login",
                quilt_rs::Error::Lineage(_) => "lineage",
                quilt_rs::Error::WorkflowValidation(_) => "workflow_validation",
                _ => "quilt_error",
            },
            Error::LoginRequired(_) => "login_required",
            Error::NamespaceNotFound(_) => "namespace_not_found",
            Error::NamespaceRequired => "namespace_required",
            Error::CommitMetaInvalid(_) => "commit_meta_invalid",
            Error::WorkflowEmpty => "workflow_empty",
            Error::WorkflowRequiresBucket => "workflow_requires_bucket",
            Error::Json(_) => "invalid_json",
            #[cfg(test)]
            Error::Test(_) => "internal",
            Error::Io(_) => "io",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    /// Nothing else pins these strings, and a consumer branching on them
    /// cannot see a rename. This table is the contract.
    #[test]
    fn error_kinds_are_stable() {
        let cases: Vec<(Error, &str)> = vec![
            (Error::Domain, "domain"),
            (Error::Home, "home"),
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
            (
                Error::LoginRequired("open.quiltdata.com".parse().expect("valid host")),
                "login_required",
            ),
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
                    "open.quiltdata.com".parse().expect("valid host"),
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
        let listing = Args::try_parse_from(["quilt", "role", "--host", "example.com"]).unwrap();
        assert!(matches!(listing.command, Commands::Role { set: None, .. }));

        let switching = Args::try_parse_from([
            "quilt",
            "role",
            "--host",
            "example.com",
            "--set",
            "ReadOnly",
        ])
        .unwrap();
        let Commands::Role { host, set } = switching.command else {
            panic!("expected the role command");
        };
        assert_eq!(host.to_string(), "example.com");
        assert_eq!(set.as_deref(), Some("ReadOnly"));
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

    /// `global = true` is what makes this additive: the shipped spelling keeps
    /// working and the new one starts working.
    #[test]
    fn json_flag_parses_before_or_after_the_subcommand() {
        let after = Args::try_parse_from(["quilt", "list", "--json"]).expect("parses");
        assert!(after.json);
        assert!(matches!(after.command, Commands::List));

        let before = Args::try_parse_from(["quilt", "--json", "list"]).expect("parses");
        assert!(before.json);
        assert!(matches!(before.command, Commands::List));

        let status = Args::try_parse_from(["quilt", "status", "-n", "demo/sales", "--json"])
            .expect("parses");
        assert!(status.json);

        let default = Args::try_parse_from(["quilt", "list"]).expect("parses");
        assert!(!default.json);
    }

    #[test]
    fn package_namespace_flag_is_optional() {
        let inferred = Args::try_parse_from(["quilt", "status"]).unwrap();
        assert!(matches!(
            inferred.command,
            Commands::Status {
                pkg: PackageRef { namespace: None },
            }
        ));

        let explicit =
            Args::try_parse_from(["quilt", "status", "--namespace", "demo/sales"]).unwrap();
        assert!(matches!(
            explicit.command,
            Commands::Status {
                pkg: PackageRef { namespace: Some(namespace) },
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
    fn test_get_default_home_dir() -> Result<(), Error> {
        let user_home = dirs::home_dir().ok_or(Error::Home)?;
        assert_eq!(
            get_default_home_dir()?,
            user_home.join(quilt_rs::DEFAULT_HOME_DIR_NAME)
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_list_uses_default_home_without_flag() -> Result<(), Error> {
        let domain_temp_dir = tempfile::tempdir()?;
        let list_args = Args {
            home: None,
            domain: Some(domain_temp_dir.path().to_path_buf()),
            verbose: false,
            json: false,
            command: Commands::List,
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
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        std::fs::write(paths.lineage(), br#"{"packages":{},"home":""}"#)?;

        initialize_home(&model, None).await?;

        assert_eq!(model.get_home().await?.as_ref(), &get_default_home_dir()?);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_existing_home_is_preserved_without_flag() -> Result<(), Error> {
        let (model, _domain_temp_dir) = Model::from_temp_dir()?;
        let home_temp_dir = tempfile::tempdir()?;
        model.set_home(home_temp_dir.path()).await?;

        initialize_home(&model, None).await?;

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
            json: false,
            command: Commands::Install {
                namespace: Some(Namespace::from(pkg::NAMESPACE).to_string()),
                uri: pkg::URI.to_string(),
                path: None,
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
            json: false,
            command: Commands::Commit {
                message: pkg::MESSAGE.to_string(),
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
                user_meta: None,
                workflow: None,
                no_workflow: true,
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(commit_args).await?;
        print(result, Format::Text, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            "New commit \"095017e53f4c8e0a07c82e562d088aa0e0f7a9ecaf2dce74a7607fac9085e98f\" created\n".to_string()
        );

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
            json: false,
            command: Commands::Commit {
                message: "Any message".to_string(),
                pkg: PackageRef {
                    namespace: Some("in/valid".to_string()),
                },
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

    /// The workflow flags are first-push-only: without `--bucket` there is no
    /// remote to set, so `--workflow` is rejected at the dispatch boundary.
    #[test(tokio::test)]
    async fn test_push_workflow_without_bucket_errors() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let push_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            verbose: false,
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: None,
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
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: None,
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

    /// With `--bucket`/`--origin` present the workflow flag is accepted: the
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
            json: false,
            command: Commands::Push {
                pkg: PackageRef {
                    namespace: Some("foo/bar".to_string()),
                },
                bucket: Some("some-bucket".to_string()),
                origin: Some(Host::from_str("open.quiltdata.com").unwrap()),
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
            json: false,
            command: Commands::Uninstall {
                pkg: PackageRef {
                    namespace: Some(pkg::NAMESPACE_STR.to_string()),
                },
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
            json: false,
            command: Commands::Uninstall {
                pkg: PackageRef {
                    namespace: Some("in/valid".to_string()),
                },
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
            json: false,
            command: Commands::List,
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
            json: false,
            command: Commands::List,
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
    async fn json_mode_emits_one_parseable_object_per_local_command() -> Result<(), Error> {
        let (_, temp_dir) = create_model_in_temp_dir().await?;
        let dir = temp_dir.path().to_path_buf();

        let args = |command| Args {
            domain: Some(dir.clone()),
            home: Some(dir.clone()),
            verbose: false,
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
                    namespace: "test/pkg".to_string(),
                    source: Some(source.clone()),
                    message: Some("first".to_string()),
                },
            ),
            ("list", Commands::List),
            ("status", Commands::Status { pkg: pkg() }),
            ("log", Commands::Log { pkg: pkg() }),
            ("uninstall", Commands::Uninstall { pkg: pkg() }),
        ];

        for (name, command) in commands {
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
            json: true,
            command: Commands::Status {
                pkg: PackageRef {
                    namespace: Some("no/such".to_string()),
                },
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
            json: false,
            command: Commands::Install {
                namespace: None,
                uri: pkg::URI.to_string(),
                path: None,
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

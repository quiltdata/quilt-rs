//! CLI frontend for `quilt_rs`. Lives under `src/bin/quilt/` so no
//! CLI types leak into the library's public API.

use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use tracing::log;

use quilt_rs::uri::Host;
use quilt_rs::uri::Namespace;

mod browse;
mod commit;
mod create;
mod install;
mod list;
mod login;
mod model;
mod output;
mod pull;
mod push;
mod status;
mod uninstall;

#[cfg(test)]
mod fixtures;

use model::Model;
pub use output::print;
pub use output::Std;

const DOMAIN_DIR_NAMESPACE: &str = "com.quiltdata.quilt-rs";

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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    command: Commands,

    /// Absolute path for the directory, where all packages will store their mutable files.
    /// Ex. /home/user/QuiltSync
    #[arg(long)]
    home: Option<PathBuf>,

    /// Path to local domain
    #[arg(short, long)]
    domain: Option<PathBuf>,
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
        /// Namespace of the package to commit new revision
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
        /// Workflow ID
        /// Ex. "my_workflow"
        #[arg(short, long)]
        workflow: Option<String>,
    },
    /// Install package locally
    Install {
        /// Source URI for the package.
        /// Ex. quilt+s3://bucket#package=foo/bar
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
    /// Authenticate with a Quilt stack
    Login {
        /// Code from the https://QUILT_STACK/code page
        #[arg(short, long)]
        code: Option<String>,
        #[arg(long)]
        host: Host,
    },
    /// List installed packages
    List,
    /// Pull
    Pull {
        /// Namespace of the package to pull
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Push
    Push {
        /// Namespace of the package to push
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
        /// S3 bucket (required for first push of local-only packages)
        #[arg(short, long, requires = "origin")]
        bucket: Option<String>,
        /// Remote host (required for first push of local-only packages)
        /// Ex. open.quiltdata.com
        #[arg(short, long, requires = "bucket")]
        origin: Option<Host>,
        // FIXME: add workflow?
    },
    /// Status of the package: modified, up-to-date, outdated
    Status {
        /// Namespace of the package. Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Uninstall package from local domain
    Uninstall {
        /// Namespace of the package to uninstall.
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
}

pub async fn init(args: Args) -> Result<Std, Error> {
    // NOTE: every command should have some domain,
    //       because domain stores credentials
    //       It's optional for user, but we use one anyway.
    //       If it is None, we use:
    //         * home directory ~/.local/share/com.quiltdata.quilt-rs`
    //         * or temporary directory
    let root_dir = get_domain_dir(args.domain)?;
    let m = Model::from(root_dir);

    // NOTE: Lineage must have home
    //       It should come either from the lineage file itself,
    //       or provided by user (when installing first time)

    if let Some(dir) = args.home {
        if let Err(err) = m.set_home(dir).await {
            log::error!("Failed to set home directory: {err}");
            return Ok(Std::Err(err));
        }
    }

    // Validate the lineage
    if let Err(err) = m.get_home().await {
        log::error!("Failed to get home directory: {err}");
        return Ok(Std::Err(err));
    }

    match args.command {
        Commands::Browse { uri } => {
            let args = browse::Input { uri };

            log::info!("Browsing {args:?}");
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

            log::info!("Creating {args:?}");
            Ok(create::command(m, args).await)
        }
        Commands::Commit {
            namespace,
            message,
            user_meta,
            workflow,
        } => {
            let user_meta = match &user_meta {
                Some(object) => match serde_json::from_str(object)? {
                    serde_json::Value::Object(object) => Some(serde_json::Value::Object(object)),
                    _ => {
                        return Err(Error::CommitMetaInvalid(object.to_string()));
                    }
                },
                None => None,
            };
            let args = commit::Input {
                message,
                namespace: namespace.try_into()?,
                user_meta,
                workflow,
                host_config: None,
            };

            log::info!("Committing {args:?}");
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

            log::info!("Installing {args:?}");
            Ok(install::command(m, args).await)
        }
        Commands::Login { code, host } => {
            if let Some(code) = code {
                let args = login::Input { code, host };

                log::info!("Logging in {args:?}");
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
        Commands::Pull { namespace } => {
            let args = pull::Input {
                namespace: namespace.try_into()?,
                host_config: None,
            };

            log::info!("Pull {args:?}");
            Ok(pull::command(m, args).await)
        }
        Commands::Push {
            namespace,
            bucket,
            origin,
        } => {
            let args = push::Input {
                namespace: namespace.try_into()?,
                host_config: None,
                bucket,
                origin,
            };

            log::info!("Pushing {args:?}");
            Ok(push::command(m, args).await)
        }
        Commands::Status { namespace } => {
            let args = status::Input {
                namespace: namespace.try_into()?,
                host_config: None,
            };

            log::info!("Status {args:?}");
            Ok(status::command(m, args).await)
        }
        Commands::Uninstall { namespace } => {
            let args = uninstall::Input {
                namespace: namespace.try_into()?,
            };

            log::info!("Uninstalling {args:?}");
            Ok(uninstall::command(m, args).await)
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Domain directory is required. We store files and credentials there")]
    Domain,

    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),

    #[error(
        r#"
Please visit https://{0}/code to get your code.
Then run:
> quilt_rs login --host {0} --code YOUR_CODE"#
    )]
    LoginRequired(Host),

    #[error("Package {0} not found")]
    NamespaceNotFound(Namespace),

    #[error("Invalid JSON for user_meta object. Object is required")]
    CommitMetaInvalid(String),

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

impl From<quilt_rs::UriError> for Error {
    fn from(err: quilt_rs::UriError) -> Error {
        Error::Quilt(quilt_rs::Error::Uri(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

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

    #[test(tokio::test)]
    async fn test_install() -> Result<(), Error> {
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
            command: Commands::Install {
                namespace: Some(Namespace::from(pkg::NAMESPACE).to_string()),
                uri: pkg::URI.to_string(),
                path: None,
            },
        };
        let mut output = Vec::new();
        let result = init(install_args).await?;
        print(result, &mut output, &mut Vec::new())?;
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
    async fn test_commit_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let commit_args = Args {
            home: Some(temp_dir.path().to_path_buf()),
            domain: Some(temp_dir.path().to_path_buf()),
            command: Commands::Commit {
                message: pkg::MESSAGE.to_string(),
                namespace: pkg::NAMESPACE_STR.to_string(),
                user_meta: None,
                workflow: None,
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(commit_args).await?;
        print(result, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            "New commit \"095017e53f4c8e0a07c82e562d088aa0e0f7a9ecaf2dce74a7607fac9085e98f\" created\n".to_string()
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_commit_invalid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let commit_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            command: Commands::Commit {
                message: "Any message".to_string(),
                namespace: "in/valid".to_string(),
                user_meta: None,
                workflow: None,
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(commit_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Package in/valid not found\n".to_string());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_pull_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::outdated as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let pull_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            command: Commands::Pull {
                namespace: pkg::NAMESPACE_STR.to_string(),
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(
            output_str,
            format!("Revision \"{}\" pulled\n", pkg::LATEST_TOP_HASH)
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
            command: Commands::Pull {
                namespace: "in/valid".to_string(),
            },
        };

        // Test init with invalid namespace
        let mut output = Vec::new();
        let result = init(pull_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Package in/valid not found\n");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_uninstall_valid() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;

        let (_, _, temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let uninstall_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            command: Commands::Uninstall {
                namespace: pkg::NAMESPACE_STR.to_string(),
            },
        };

        // Test init with valid arguments
        let mut output = Vec::new();
        let result = init(uninstall_args).await?;
        print(result, &mut output, &mut Vec::new())?;
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
            command: Commands::Uninstall {
                namespace: "in/valid".to_string(),
            },
        };

        // Test init with invalid namespace
        let mut output = Vec::new();
        let result = init(uninstall_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
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
            command: Commands::List,
        };

        // Test init with invalid permissions
        let mut output = Vec::new();
        let result = init(list_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Permission denied"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_list_valid() -> Result<(), Error> {
        // Create temporary directory for domain
        let (_, temp_dir) = create_model_in_temp_dir().await?;

        let list_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            command: Commands::List {},
        };

        // Test init with empty domain
        let mut output = Vec::new();
        let result = init(list_args).await?;
        print(result, &mut output, &mut Vec::new())?;
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "No installed packages\n");

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
            command: Commands::Install {
                namespace: None,
                uri: pkg::URI.to_string(),
                path: None,
            },
        };

        // Test init with invalid URI
        let mut output = Vec::new();
        let result = init(install_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
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
    async fn test_browse_valid() -> Result<(), Error> {
        use crate::cli::fixtures::get_browse_output;
        use crate::cli::fixtures::packages::default as pkg;

        // Create temporary directory for domain
        let temp_dir = tempfile::tempdir()?;
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);

        let browse_args = Args {
            domain: Some(temp_dir.path().to_path_buf()),
            home: Some(temp_dir.path().to_path_buf()),
            command: Commands::Browse { uri },
        };

        // Test init with valid URI
        let mut output = Vec::new();
        let result = init(browse_args).await?;
        print(result, &mut output, &mut Vec::new())?;
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
            command: Commands::Browse {
                uri: pkg::URI.to_string(),
            },
        };

        // Test init with invalid URI
        let mut output = Vec::new();
        let result = init(browse_args).await?;
        print(result, &mut Vec::new(), &mut output)?;
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

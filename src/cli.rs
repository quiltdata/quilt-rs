//! Not a part of the library and meant to be an independent project.
//! This is a CLI frontend for `quilt_rs`.

use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use tempfile::TempDir;
use tracing::log;

use quilt_rs::uri::Host;
use quilt_rs::uri::Namespace;

mod benchmark;
mod browse;
mod commit;
mod install;
mod list;
mod login;
mod model;
mod output;
mod package;
mod pull;
mod push;
mod status;
mod uninstall;

use model::Model;
use output::print;
use output::Std;

const DOMAIN_DIR_NAMESPACE: &str = "com.quiltdata.quilt-rs";

fn parse_optional_namespace(namespace: Option<String>) -> Result<Option<Namespace>, Error> {
    Ok(match namespace {
        Some(namespace) => Some(namespace.try_into()?),
        None => None,
    })
}

fn get_domain_dir(dir_arg: Option<PathBuf>) -> Result<(PathBuf, Option<TempDir>), Error> {
    Ok(match dir_arg {
        Some(user_specified_dir) => (user_specified_dir, None),
        None => match dirs::data_local_dir() {
            Some(default_user_dir) => (default_user_dir.join(DOMAIN_DIR_NAMESPACE), None),
            None => {
                let temp_dir = TempDir::new()?;
                (temp_dir.path().to_path_buf(), Some(temp_dir))
            }
        },
    })
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse remote manifest
    Browse {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        #[arg(value_name = "PKG_URI")]
        uri: String,
    },
    /// Commit new package revision
    Commit {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
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
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
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
    /// List installed packages
    Login {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Code from the https://QUILT_STACK/code page
        #[arg(short, long)]
        code: Option<String>,
        #[arg(long)]
        host: Host,
    },
    /// List installed packages
    List {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
    },
    /// Create and install manifest to S3
    Package {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Commit message
        #[arg(short, long)]
        message: Option<String>,
        /// Source URI for the package.
        /// Ex. s3://bucket/s3/prefix
        #[arg(value_name = "S3_URI")]
        uri: String,
        /// quilt+s3 URI for new package
        #[arg(short, long, value_name = "PKG_URI")]
        target: String,
        /// JSON string for user meta
        #[arg(short, long)]
        user_meta: Option<String>,
    },
    /// Pull
    Pull {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Namespace of the package to pull
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Push
    Push {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Namespace of the package to push
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
        // FIXME: add workflow?
    },
    /// Status of the package: modified, up-to-date, outdated
    Status {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Namespace of the package. Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Test and benchmark creating manifest with large number of rows
    Benchmark {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// How many rows in manifest?
        /// Ex. 1000000
        #[arg(short, long)]
        number: i32,
        /// Manifest destination path
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Uninstall package from local domain
    Uninstall {
        /// Path to local domain
        #[arg(short, long)]
        domain: Option<PathBuf>,
        /// Namespace of the package to uninstall.
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
}

// TODO: pass args as an argument, so we can test it
pub async fn init() -> Result<(), Error> {
    let args = Args::parse();

    // FIXME: every command should have a domain,
    //        because domain stores credentials
    match args.command {
        Commands::Browse { domain, uri } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = browse::Input { uri };

            log::info!("Browsing {:?}", args);
            print(browse::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Commit {
            domain,
            namespace,
            message,
            user_meta,
            workflow,
        } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);

            let user_meta = match &user_meta {
                Some(object) => match serde_json::from_str(object)? {
                    serde_json::Value::Object(object) => Some(object),
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
            };

            log::info!("Committing {:?}", args);
            print(commit::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Install {
            path,
            domain,
            namespace,
            uri,
        } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = install::Input {
                namespace: parse_optional_namespace(namespace)?,
                paths: path,
                uri,
            };

            log::info!("Installing {:?}", args);
            print(install::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Login { code, domain, host } => {
            if let Some(code) = code {
                let (root_dir, temp_dir) = get_domain_dir(domain)?;
                let m = Model::from(root_dir);
                let args = login::Input { code, host };

                log::info!("Logging in {:?}", args);
                print(login::command(m, args).await);

                if let Some(temp_dir) = temp_dir {
                    log::warn!("Temporary domain was used: {:?}", temp_dir);
                }
            } else {
                // TODO: Check the lineage, if there are some `package.remote.catalog`
                print(Std::Err(Error::LoginRequired(format!(
                    r#"
Please visit https://{0}/code to get your code.
Then run:
> quilt_rs login --host {0} --code YOUR_CODE"#,
                    host
                ))));
            }
            Ok(())
        }
        Commands::List { domain } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);

            log::info!("Listing installed packages");
            print(list::command(m).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Package {
            domain,
            message,
            target,
            uri,
            user_meta,
        } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let user_meta = match &user_meta {
                Some(object) => match serde_json::from_str(object)? {
                    serde_json::Value::Object(object) => Some(object),
                    _ => {
                        return Err(Error::CommitMetaInvalid(object.to_string()));
                    }
                },
                None => None,
            };
            let args = package::Input {
                message,
                target,
                uri,
                user_meta,
            };

            log::info!("Packaging {:?}", args);
            print(package::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Pull { domain, namespace } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = pull::Input {
                namespace: namespace.try_into()?,
            };

            log::info!("Pull {:?}", args);
            print(pull::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Push { domain, namespace } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = push::Input {
                namespace: namespace.try_into()?,
            };

            log::info!("Pushing {:?}", args);
            print(push::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Status { domain, namespace } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = status::Input {
                namespace: namespace.try_into()?,
            };

            log::info!("Status {:?}", args);
            print(status::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Benchmark {
            domain,
            number,
            path,
        } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = benchmark::Input {
                number,
                dest: path.unwrap_or(PathBuf::from("manifest.pq")),
            };

            log::info!("Benchmark manifest creation {:?}", args,);
            print(benchmark::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
        Commands::Uninstall { domain, namespace } => {
            let (root_dir, temp_dir) = get_domain_dir(domain)?;
            let m = Model::from(root_dir);
            let args = uninstall::Input {
                namespace: namespace.try_into()?,
            };

            log::info!("Uninstalling {:?}", args);
            print(uninstall::command(m, args).await);

            if let Some(temp_dir) = temp_dir {
                log::warn!("Temporary domain was used: {:?}", temp_dir);
            }

            Ok(())
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),

    #[error("Login required: {0}")]
    LoginRequired(String), // TODO: Host?

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

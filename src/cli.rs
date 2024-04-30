use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use tracing::log;

use quilt_rs::uri::Namespace;

mod browse;
mod commit;
mod install;
mod list;
mod model;
mod output;
mod package;
mod pull;
mod push;
mod status;
mod uninstall;

use model::Model;
use output::print;

fn parse_optional_namespace(namespace: Option<String>) -> Result<Option<Namespace>, Error> {
    Ok(match namespace {
        Some(namespace) => Some(namespace.try_into()?),
        None => None,
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
        #[arg(value_name = "PKG_URI")]
        uri: String,
    },
    /// Commit new package revision
    Commit {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
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
    },
    /// Install package locally
    Install {
        /// Source URI for the package.
        /// Ex. quilt+s3://bucket#package=foo/bar
        #[arg(value_name = "PKG_URI")]
        uri: String,
        /// Path to local domain. Should be absolute path when installing paths
        #[arg(short, long)]
        domain: PathBuf,
        /// Namespace for the package, ex. foo/bar.
        #[arg(short, long)]
        namespace: Option<String>,
        /// Logical key relative to the root of the package to be installed locally.
        /// You can provide multiple paths.
        #[arg(short, long)]
        path: Option<Vec<PathBuf>>,
    },
    /// List installed packages
    List {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
    },
    /// Create and install manifest to S3
    Package {
        /// Source URI for the package.
        /// Ex. s3://bucket/s3/prefix
        #[arg(value_name = "S3_URI")]
        uri: String,
        /// quilt+s3 URI for new package
        #[arg(short, long, value_name = "PKG_URI")]
        target: String,
    },
    /// Pull
    Pull {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
        /// Namespace of the package to pull
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Push
    Push {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
        /// Namespace of the package to push
        /// Ex. foo/bar
        #[arg(short, long)]
        namespace: String,
    },
    /// Status of the package: modified, up-to-date, outdated
    Status {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
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
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
    },
}

pub async fn init() -> Result<(), Error> {
    let args = Args::parse();

    match args.command {
        Commands::Browse { uri } => {
            let (m, temp_dir) = Model::from_temp_dir()?;
            let args = browse::Input { uri };
            log::info!("Browsing {:?} using {:?}", args, temp_dir);
            print(browse::command(m, args).await);
            Ok(())
        }
        Commands::Commit {
            domain,
            namespace,
            message,
            user_meta,
        } => {
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
            };
            log::info!("Committing {:?}", args);
            print(commit::command(Model::from(domain), args).await);
            Ok(())
        }
        Commands::Install {
            path,
            domain,
            namespace,
            uri,
        } => {
            let args = install::Input {
                namespace: parse_optional_namespace(namespace)?,
                paths: path,
                uri,
            };
            log::info!("Installing {:?}", args);
            print(install::command(Model::from(domain), args).await);
            Ok(())
        }
        Commands::List { domain } => {
            if !domain.exists() {
                return Err(Error::Domain(domain));
            }
            log::info!("Listing installed packages");
            print(list::command(Model::from(domain)).await);
            Ok(())
        }
        Commands::Package { uri, target } => {
            let (m, temp_dir) = Model::from_temp_dir()?;
            let args = package::Input { target, uri };
            log::info!("Packaging {:?} using {:?}", args, temp_dir);
            print(package::command(m, args).await);
            Ok(())
        }
        Commands::Pull { domain, namespace } => {
            let args = pull::Input {
                namespace: namespace.try_into()?,
            };
            log::info!("Pull {:?}", args);
            print(pull::command(Model::from(domain), args).await);
            Ok(())
        }
        Commands::Push { domain, namespace } => {
            let args = push::Input {
                namespace: namespace.try_into()?,
            };
            log::info!("Pushing {:?}", args);
            print(push::command(Model::from(domain), args).await);
            Ok(())
        }
        Commands::Status { domain, namespace } => {
            let args = status::Input {
                namespace: namespace.try_into()?,
            };
            log::info!("Status {:?}", args);
            print(status::command(Model::from(domain), args).await);
            Ok(())
        }
        Commands::Uninstall { domain, namespace } => {
            if !domain.exists() {
                return Err(Error::Domain(domain));
            }
            let args = uninstall::Input {
                namespace: namespace.try_into()?,
            };
            log::info!("Uninstalling {:?}", args);
            print(uninstall::command(Model::from(domain), args).await);
            Ok(())
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Domain path doesn't exists: {0}")]
    Domain(PathBuf),

    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),

    #[error("Failed to create temp dir: {0}")]
    TempDir(String),

    #[error("Package {0} not found")]
    NamespaceNotFound(Namespace),

    #[error("Invalid JSON for user_meta object. Object is required")]
    CommitMetaInvalid(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<quilt_rs::Error> for Error {
    fn from(err: quilt_rs::Error) -> Error {
        Error::Quilt(err)
    }
}

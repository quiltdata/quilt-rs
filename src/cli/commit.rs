use quilt_rs::lineage::CommitState;
use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub message: String,
    pub namespace: Namespace,
    pub user_meta: Option<quilt_rs::manifest::JsonObject>,
}

#[derive(Debug)]
pub struct Output {
    pub hash: Option<String>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.hash {
            Some(hash) => {
                write!(f, r##"New commit "{}" created"##, hash)
            }
            None => write!(f, "Nothing commited"),
        }
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.commit(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn commit_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    message: String,
    user_meta: Option<quilt_rs::manifest::JsonObject>,
) -> Result<Option<CommitState>, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.commit(message, user_meta).await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        message,
        namespace,
        user_meta,
    }: Input,
) -> Result<Output, Error> {
    let commit_state = commit_package(local_domain, namespace, message, user_meta).await?;
    Ok(Output {
        hash: commit_state.map(|s| s.hash),
    })
}

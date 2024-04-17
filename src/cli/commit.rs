use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub message: String,
    pub namespace: String,
}

#[derive(Debug)]
pub struct Output {}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = vec!["Commit"];
        write!(f, "{}", output.join("\n"))
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
    namespace: String,
    message: String,
    user_meta: Option<quilt_rs::quilt::manifest::JsonObject>,
) -> Result<(), Error> {
    let installed_package = local_domain.get_installed_package(&namespace).await?;

    match installed_package {
        Some(installed_package) => {
            installed_package.commit(message, user_meta).await?;
            Ok(())
        }
        None => Err(Error::NamespaceNotFound(namespace.to_string())),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { message, namespace }: Input,
) -> Result<Output, Error> {
    commit_package(local_domain, namespace, message, None).await?;
    Ok(Output {})
}

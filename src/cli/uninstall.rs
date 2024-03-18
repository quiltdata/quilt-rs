use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: String,
}

pub struct Output {
    namespace: String,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Package {} successfully uninstalled", self.namespace)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.uninstall(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,

    Input { namespace }: Input,
) -> Result<Output, Error> {
    local_domain.uninstall_package(&namespace).await?;
    Ok(Output { namespace })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore]
    async fn uninstall() -> Result<(), String> {
        unreachable!()
    }
}

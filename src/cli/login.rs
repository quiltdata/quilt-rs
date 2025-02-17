use quilt_rs::uri::Host;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub code: String,
    pub host: Host,
}

pub struct Output {
    host: Host,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        output.push(format!("Successfully logged in to {}", self.host));
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.login(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { code, host }: Input,
) -> Result<Output, Error> {
    let remote = local_domain.get_remote();
    remote.login(&host, code).await?;
    Ok(Output { host })
}

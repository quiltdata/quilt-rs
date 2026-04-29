use quilt_uri::Host;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

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
    Std::from_result(m.login(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { code, host }: Input,
) -> Result<Output, Error> {
    let remote = local_domain.get_remote();
    remote.login(&host, code).await?;
    Ok(Output { host })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;
    use test_log::test;

    #[test]
    fn test_output_display() {
        let host = Host::from_str("example.com").unwrap();
        let output = Output { host };

        let display_string = format!("{output}");
        assert_eq!(display_string, "Successfully logged in to example.com");
    }
}

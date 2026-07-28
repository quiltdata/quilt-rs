use quilt_rs::RoleInfo;
use quilt_uri::Host;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub host: Host,
    /// Role to switch to. `None` only reports the roles you hold.
    pub set: Option<String>,
}

pub struct Output {
    info: RoleInfo,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.info.available.is_empty() {
            return write!(f, "No roles available");
        }
        let mut output: Vec<String> = Vec::new();
        for role in &self.info.available {
            let marker = if *role == self.info.current { '*' } else { ' ' };
            output.push(format!("{marker} {role}"));
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.role(args).await)
}

/// Report the roles the user holds on `host`, or make `set` the active one.
///
/// A switch is server-side and global: it changes what every Quilt client
/// signed in as this user may read and write, not just this process.
///
/// The desktop app has to clear its cached S3 clients right after a switch,
/// because an already-built client keeps signing with the previous role's
/// credentials until they expire. **The CLI has no such cache to clear.**
/// Every invocation is a fresh process, and it builds its clients after the
/// switch has already landed — so there is deliberately no
/// `clear_client_cache` call here to mirror the desktop path.
pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { host, set }: Input,
) -> Result<Output, Error> {
    let remote = local_domain.get_remote();
    let info = match &set {
        Some(role_name) => remote.switch_role(&host, role_name).await?,
        None => remote.refresh_roles(&host).await?,
    };
    Ok(Output { info })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test]
    fn output_marks_the_active_role() {
        let output = Output {
            info: RoleInfo {
                current: "ReadWrite".to_string(),
                available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
            },
        };

        let shown = format!("{output}");
        assert!(
            shown.contains("* ReadWrite"),
            "active role must be marked: {shown}"
        );
        assert!(
            shown.contains("  ReadOnly"),
            "held roles must be listed: {shown}"
        );
    }

    /// The marker must follow `current`, not the first line: a stack that
    /// lists the active role second would otherwise be rendered wrongly
    /// while the test above still passed.
    #[test]
    fn output_marks_the_active_role_wherever_it_appears() {
        let output = Output {
            info: RoleInfo {
                current: "ReadOnly".to_string(),
                available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
            },
        };

        assert_eq!(format!("{output}"), "  ReadWrite\n* ReadOnly");
    }

    #[test]
    fn output_without_roles_says_so() {
        let output = Output {
            info: RoleInfo {
                current: String::new(),
                available: Vec::new(),
            },
        };

        assert_eq!(format!("{output}"), "No roles available");
    }
}

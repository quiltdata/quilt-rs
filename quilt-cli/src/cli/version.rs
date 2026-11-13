use std::path::PathBuf;

use clap::CommandFactory;

use crate::cli::Args;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub details: bool,
    /// The domain this run uses, `None` when no default could be found.
    pub domain: Option<PathBuf>,
    pub domain_from: DomainFrom,
}

/// Where the domain this run uses came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DomainFrom {
    #[default]
    Default,
    Env,
    Flag,
}

impl DomainFrom {
    pub fn from_source(source: Option<clap::parser::ValueSource>) -> Self {
        match source {
            Some(clap::parser::ValueSource::CommandLine) => DomainFrom::Flag,
            Some(clap::parser::ValueSource::EnvVariable) => DomainFrom::Env,
            _ => DomainFrom::Default,
        }
    }
}

impl std::fmt::Display for DomainFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DomainFrom::Default => "default",
            DomainFrom::Env => crate::cli::DOMAIN_ENV,
            DomainFrom::Flag => "--domain",
        })
    }
}

pub struct Output {
    details: Option<Details>,
}

struct Details {
    domain: Option<PathBuf>,
    domain_from: DomainFrom,
    default_domain: Option<PathBuf>,
    /// `Ok(None)` when the domain has no home yet.
    home: Result<Option<PathBuf>, String>,
}

/// The line `quilt --version` prints, so the two never differ.
fn version_line() -> String {
    Args::command().render_version().trim_end().to_string()
}

fn path_text(path: Option<&PathBuf>) -> String {
    path.map_or_else(|| "unknown".to_string(), |path| path.display().to_string())
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", version_line())?;
        let Some(details) = &self.details else {
            return Ok(());
        };
        writeln!(f)?;
        writeln!(f, "quilt-rs: {}", quilt_rs::VERSION)?;
        writeln!(f, "quilt-uri: {}", quilt_uri::VERSION)?;
        writeln!(f, "os: {}", std::env::consts::OS)?;
        writeln!(f, "arch: {}", std::env::consts::ARCH)?;
        writeln!(f, "domain: {}", path_text(details.domain.as_ref()))?;
        writeln!(f, "domain-from: {}", details.domain_from)?;
        writeln!(
            f,
            "default-domain: {}",
            path_text(details.default_domain.as_ref())
        )?;
        match &details.home {
            Ok(Some(home)) => write!(f, "home: {}", home.display()),
            Ok(None) => write!(f, "home: not set"),
            Err(err) => write!(f, "home: unreadable: {err}"),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({ "version": env!("CARGO_PKG_VERSION") });
        let Some(details) = &self.details else {
            return json;
        };
        let path_json =
            |path: Option<&PathBuf>| path.map(|path| path.to_string_lossy().to_string());
        json["quilt_rs"] = quilt_rs::VERSION.into();
        json["quilt_uri"] = quilt_uri::VERSION.into();
        json["os"] = std::env::consts::OS.into();
        json["arch"] = std::env::consts::ARCH.into();
        json["domain"] = path_json(details.domain.as_ref()).into();
        json["domain_from"] = details.domain_from.to_string().into();
        json["default_domain"] = path_json(details.default_domain.as_ref()).into();
        match &details.home {
            Ok(home) => json["home"] = path_json(home.as_ref()).into(),
            Err(err) => {
                json["home"] = serde_json::Value::Null;
                json["home_error"] = err.as_str().into();
            }
        }
        json
    }
}

/// Reads the domain and never writes to it: a domain that does not exist
/// stays that way.
pub async fn command(args: Input) -> Std {
    let details = if args.details {
        let home = match &args.domain {
            Some(domain) => quilt_rs::LocalDomain::new(domain)
                .stored_home()
                .await
                .map(|home| home.map(|home| home.as_ref().clone()))
                .map_err(|err| err.to_string()),
            None => Ok(None),
        };
        Some(Details {
            domain: args.domain,
            domain_from: args.domain_from,
            default_domain: crate::cli::default_domain_dir(),
            home,
        })
    } else {
        None
    };
    Std::Out(Box::new(Output { details }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::CommandFactory;
    use clap::Parser;
    use test_log::test;

    use super::DomainFrom;
    use crate::cli::Args;
    use crate::cli::Commands;
    use crate::cli::Error;
    use crate::cli::Format;
    use crate::cli::init;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::print;

    fn version_args(domain: &Path, details: bool) -> Args {
        Args {
            home: None,
            domain: Some(domain.to_path_buf()),
            verbose: false,
            log: None,
            format: None,
            json: false,
            command: Commands::Version {
                details,
                domain_from: DomainFrom::Flag,
            },
        }
    }

    async fn run(args: Args, format: Format) -> Result<String, Error> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        print(init(args).await?, format, &mut stdout, &mut stderr)?;
        assert_eq!(String::from_utf8(stderr).expect("utf8"), "");
        Ok(String::from_utf8(stdout).expect("utf8"))
    }

    #[test(tokio::test)]
    async fn prints_what_dash_dash_version_prints() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;

        let stdout = run(version_args(domain.path(), false), Format::Text).await?;

        assert_eq!(stdout, Args::command().render_version());
        assert_eq!(stdout, format!("quilt-cli {}\n", env!("CARGO_PKG_VERSION")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn prints_the_version_as_json() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;

        let stdout = run(version_args(domain.path(), false), Format::Json).await?;

        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
        assert_eq!(
            parsed,
            serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn details_print_the_stored_home() -> Result<(), Error> {
        let (_m, temp_dir) = create_model_in_temp_dir().await?;

        let stdout = run(version_args(temp_dir.path(), true), Format::Text).await?;

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines[0], format!("quilt-cli {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(lines[1], format!("quilt-rs: {}", quilt_rs::VERSION));
        assert_eq!(lines[2], format!("quilt-uri: {}", quilt_uri::VERSION));
        assert_eq!(lines[3], format!("os: {}", std::env::consts::OS));
        assert_eq!(lines[4], format!("arch: {}", std::env::consts::ARCH));
        assert_eq!(lines[5], format!("domain: {}", temp_dir.path().display()));
        assert_eq!(lines[6], "domain-from: --domain");
        assert!(lines[7].starts_with("default-domain: "), "{stdout}");
        assert_eq!(lines[8], format!("home: {}", temp_dir.path().display()));
        assert_eq!(lines.len(), 9, "{stdout}");
        Ok(())
    }

    /// A bug report must not change what it reports on: no domain folder, no
    /// default home.
    #[test(tokio::test)]
    async fn details_on_a_missing_domain_create_nothing() -> Result<(), Error> {
        let parent = tempfile::tempdir()?;
        let domain = parent.path().join("missing");

        let stdout = run(version_args(&domain, true), Format::Text).await?;

        assert!(stdout.ends_with("home: not set\n"), "{stdout}");
        assert!(!domain.exists(), "the domain is not created");
        Ok(())
    }

    #[test(tokio::test)]
    async fn details_on_an_empty_domain_write_nothing() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;

        let stdout = run(version_args(domain.path(), true), Format::Json).await?;

        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
        assert_eq!(parsed["home"], serde_json::Value::Null);
        assert_eq!(
            std::fs::read_dir(domain.path())?.count(),
            0,
            "nothing written"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn details_as_json_carry_the_same_fields() -> Result<(), Error> {
        let (_m, temp_dir) = create_model_in_temp_dir().await?;

        let stdout = run(version_args(temp_dir.path(), true), Format::Json).await?;

        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
        let default_domain = crate::cli::default_domain_dir()
            .map(|dir| serde_json::Value::from(dir.to_string_lossy()));
        assert_eq!(
            parsed,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "quilt_rs": quilt_rs::VERSION,
                "quilt_uri": quilt_uri::VERSION,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "domain": temp_dir.path().to_string_lossy(),
                "domain_from": "--domain",
                "default_domain": default_domain,
                "home": temp_dir.path().to_string_lossy(),
            })
        );
        Ok(())
    }

    /// A domain whose record does not parse still gets a report.
    #[test(tokio::test)]
    async fn details_report_an_unreadable_home() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;
        let paths = quilt_rs::paths::DomainPaths::new(domain.path().to_path_buf());
        let lineage = paths.lineage();
        std::fs::create_dir_all(lineage.parent().expect("a parent"))?;
        std::fs::write(&lineage, b"not json")?;

        let stdout = run(version_args(domain.path(), true), Format::Text).await?;

        assert!(
            stdout
                .lines()
                .last()
                .is_some_and(|line| line.starts_with("home: unreadable: ")),
            "{stdout}"
        );
        Ok(())
    }

    #[test]
    fn version_skips_the_update_check() {
        let args = Args::try_parse_from(["quilt", "version", "--details"]).expect("parses");
        assert!(!args.shows_update_notice());
    }

    /// `-v` stays the global logs flag; it does not ask for details.
    #[test]
    fn verbose_does_not_mean_details() {
        let args = Args::try_parse_from(["quilt", "version", "-v"]).expect("parses");
        assert!(args.verbose);
        assert!(matches!(
            args.command,
            Commands::Version { details: false, .. }
        ));
    }
}

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::lineage::Home;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    /// `None` prints the current home.
    pub dir: Option<PathBuf>,
    pub overwrite: bool,
}

pub struct Output {
    home: Home,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.home.as_ref().display())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "home": self.home.as_ref().to_string_lossy() })
    }
}

/// What a refused change tells the reader to run instead: the flag on
/// `quilt home <dir>`, or the whole command for the deprecated `--home`,
/// which has no `--overwrite` of its own.
#[derive(Debug)]
pub enum Retry {
    OverwriteFlag,
    Command(PathBuf),
}

impl std::fmt::Display for Retry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Retry::OverwriteFlag => write!(f, "Pass --overwrite to reset it anyway."),
            Retry::Command(dir) => write!(
                f,
                "Run \"quilt home {} --overwrite\" to reset it anyway.",
                dir.display()
            ),
        }
    }
}

/// Who asked for the change, which decides how a refusal says to retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// `quilt home <dir>`
    Subcommand,
    /// The deprecated global `--home <dir>`
    HomeFlag,
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.home(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { dir, overwrite }: Input,
) -> Result<Output, Error> {
    let home = match dir {
        None => local_domain.get_home().await?,
        Some(dir) => set(local_domain, &dir, overwrite, Via::Subcommand).await?,
    };
    Ok(Output { home })
}

/// Store `dir`, resolved against the current directory, as the home and
/// create its folder.
pub async fn set(
    local_domain: &quilt_rs::LocalDomain,
    dir: &Path,
    overwrite: bool,
    via: Via,
) -> Result<Home, Error> {
    let dir = resolve(dir, &std::env::current_dir()?)?;
    let result = if overwrite {
        local_domain.overwrite_home(&dir).await
    } else {
        local_domain.set_home(&dir).await
    };
    let home = result.map_err(|err| match err {
        err @ quilt_rs::Error::Lineage(quilt_rs::LineageError::HomeInUse { .. }) => {
            Error::HomeInUse {
                reason: err.to_string(),
                retry: match via {
                    Via::Subcommand => Retry::OverwriteFlag,
                    Via::HomeFlag => Retry::Command(dir.clone()),
                },
            }
        }
        err => Error::from(err),
    })?;
    std::fs::create_dir_all(home.as_ref())?;
    Ok(home)
}

/// `dir` as an absolute path, a relative one taken against `base`, with `.`
/// and `..` folded away so one folder is stored one way.
///
/// Lexical, not `canonicalize`: resolving symlinks would make a stored home
/// that names a link (`/tmp` on macOS) read as a different folder.
fn resolve(dir: &Path, base: &Path) -> Result<PathBuf, Error> {
    let absolute = std::path::absolute(base.join(dir))?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use test_log::test;

    use crate::cli::Args;
    use crate::cli::Commands;
    use crate::cli::Error;
    use crate::cli::Format;
    use crate::cli::get_default_home_dir;
    use crate::cli::init;
    use crate::cli::initialize_home;
    use crate::cli::model::Commands as _;
    use crate::cli::model::Model;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::print;

    fn home_args(domain: &std::path::Path, dir: Option<PathBuf>, overwrite: bool) -> Args {
        Args {
            home: None,
            domain: Some(domain.to_path_buf()),
            verbose: false,
            json: false,
            command: Commands::Home { dir, overwrite },
        }
    }

    /// Runs `args` and returns (stdout, stderr) as `print` would write them.
    async fn run(args: Args, format: Format) -> Result<(String, String), Error> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        print(init(args).await?, format, &mut stdout, &mut stderr)?;
        Ok((
            String::from_utf8(stdout).expect("utf8"),
            String::from_utf8(stderr).expect("utf8"),
        ))
    }

    async fn stored_home(domain: &std::path::Path) -> Result<PathBuf, Error> {
        Ok(quilt_rs::LocalDomain::new(domain)
            .get_home()
            .await?
            .as_ref()
            .clone())
    }

    /// A domain with one local package installed under `temp_dir`.
    async fn domain_with_one_package() -> Result<(Model, tempfile::TempDir), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;
        m.create(crate::cli::create::Input {
            namespace: ("example", "local").into(),
            source: None,
            message: None,
        })
        .await?;
        Ok((m, temp_dir))
    }

    fn data_json(domain: &std::path::Path) -> Vec<u8> {
        let paths = quilt_rs::paths::DomainPaths::new(domain.to_path_buf());
        std::fs::read(paths.lineage()).expect("data.json exists")
    }

    #[test(tokio::test)]
    async fn prints_the_current_home() -> Result<(), Error> {
        let (_m, temp_dir) = create_model_in_temp_dir().await?;

        let (stdout, stderr) = run(home_args(temp_dir.path(), None, false), Format::Text).await?;

        assert_eq!(stdout, format!("{}\n", temp_dir.path().display()));
        assert_eq!(stderr, "");
        Ok(())
    }

    #[test(tokio::test)]
    async fn prints_the_home_as_json() -> Result<(), Error> {
        let (_m, temp_dir) = create_model_in_temp_dir().await?;

        let (stdout, _) = run(home_args(temp_dir.path(), None, false), Format::Json).await?;

        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
        assert_eq!(
            parsed,
            serde_json::json!({ "home": temp_dir.path().to_string_lossy() })
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn on_an_empty_domain_sets_and_prints_the_default() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;

        let (stdout, _) = run(home_args(domain.path(), None, false), Format::Text).await?;

        let default = get_default_home_dir()?;
        assert_eq!(stdout, format!("{}\n", default.display()));
        assert_eq!(stored_home(domain.path()).await?, default);
        Ok(())
    }

    #[test(tokio::test)]
    async fn sets_an_absolute_home_and_creates_the_folder() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;
        let parent = tempfile::tempdir()?;
        let new_home = parent.path().join("not/yet/there");

        let (stdout, _) = run(
            home_args(domain.path(), Some(new_home.clone()), false),
            Format::Text,
        )
        .await?;

        assert_eq!(stdout, format!("{}\n", new_home.display()));
        assert_eq!(stored_home(domain.path()).await?, new_home);
        assert!(new_home.is_dir(), "the folder is created");
        Ok(())
    }

    #[test(tokio::test)]
    async fn sets_a_relative_home_against_the_current_directory() -> Result<(), Error> {
        let domain = tempfile::tempdir()?;

        run(
            home_args(domain.path(), Some(PathBuf::from(".")), false),
            Format::Text,
        )
        .await?;

        assert_eq!(stored_home(domain.path()).await?, std::env::current_dir()?);
        Ok(())
    }

    #[test]
    fn resolve_joins_a_relative_dir_to_the_base() {
        let base = std::path::Path::new("/work/dir");
        assert_eq!(
            super::resolve(std::path::Path::new("."), base).unwrap(),
            PathBuf::from("/work/dir")
        );
        assert_eq!(
            super::resolve(std::path::Path::new("sub/home"), base).unwrap(),
            PathBuf::from("/work/dir/sub/home")
        );
        assert_eq!(
            super::resolve(std::path::Path::new("/abs/home"), base).unwrap(),
            PathBuf::from("/abs/home")
        );
        assert_eq!(
            super::resolve(std::path::Path::new("../Data"), base).unwrap(),
            PathBuf::from("/work/Data")
        );
        assert_eq!(
            super::resolve(std::path::Path::new("sub/../home"), base).unwrap(),
            PathBuf::from("/work/dir/home")
        );
    }

    #[test(tokio::test)]
    async fn refuses_a_new_home_while_packages_are_installed() -> Result<(), Error> {
        let (_m, temp_dir) = domain_with_one_package().await?;
        let before = data_json(temp_dir.path());
        let other = tempfile::tempdir()?;

        let (stdout, stderr) = run(
            home_args(temp_dir.path(), Some(other.path().to_path_buf()), false),
            Format::Text,
        )
        .await?;

        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            format!(
                "Cannot change the home: 1 package is installed in {}. \
                 Moving the home isn't supported yet. Pass --overwrite to reset it anyway.\n",
                temp_dir.path().display()
            )
        );
        assert_eq!(data_json(temp_dir.path()), before, "data.json unchanged");
        Ok(())
    }

    #[test(tokio::test)]
    async fn refusal_as_json_has_its_own_kind() -> Result<(), Error> {
        let (_m, temp_dir) = domain_with_one_package().await?;
        let other = tempfile::tempdir()?;

        let (_, stderr) = run(
            home_args(temp_dir.path(), Some(other.path().to_path_buf()), false),
            Format::Json,
        )
        .await?;

        let parsed: serde_json::Value = serde_json::from_str(&stderr).expect("JSON");
        assert_eq!(parsed["error"]["kind"], "home_in_use");
        Ok(())
    }

    #[test(tokio::test)]
    async fn overwrite_changes_the_home_while_packages_are_installed() -> Result<(), Error> {
        let (_m, temp_dir) = domain_with_one_package().await?;
        let other = tempfile::tempdir()?;

        run(
            home_args(temp_dir.path(), Some(other.path().to_path_buf()), true),
            Format::Text,
        )
        .await?;

        assert_eq!(stored_home(temp_dir.path()).await?, other.path());
        Ok(())
    }

    #[test]
    fn overwrite_needs_a_dir() {
        use clap::Parser;
        assert!(Args::try_parse_from(["quilt", "home", "--overwrite"]).is_err());
        let args = Args::try_parse_from(["quilt", "home", "/x", "--overwrite"]).unwrap();
        assert!(matches!(
            args.command,
            Commands::Home {
                dir: Some(_),
                overwrite: true
            }
        ));
    }

    #[test(tokio::test)]
    async fn home_flag_warns_and_sets_the_home_on_an_empty_domain() -> Result<(), Error> {
        let (m, _domain) = Model::from_temp_dir()?;
        let new_home = tempfile::tempdir()?;
        let mut stderr = Vec::new();

        initialize_home(&m, Some(new_home.path().to_path_buf()), &mut stderr).await?;

        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "warning: --home is deprecated; use \"quilt home <dir>\"\n"
        );
        assert_eq!(m.get_home().await?.as_ref(), new_home.path());
        Ok(())
    }

    #[test(tokio::test)]
    async fn home_flag_naming_the_current_home_changes_nothing() -> Result<(), Error> {
        let (m, temp_dir) = domain_with_one_package().await?;
        let before = data_json(temp_dir.path());

        initialize_home(&m, Some(temp_dir.path().to_path_buf()), &mut Vec::new()).await?;

        assert_eq!(data_json(temp_dir.path()), before);
        Ok(())
    }

    #[test(tokio::test)]
    async fn home_flag_refusal_points_at_the_subcommand() -> Result<(), Error> {
        let (m, temp_dir) = domain_with_one_package().await?;
        let other = tempfile::tempdir()?;

        let err = initialize_home(&m, Some(other.path().to_path_buf()), &mut Vec::new())
            .await
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!(
                "Cannot change the home: 1 package is installed in {}. \
                 Moving the home isn't supported yet. \
                 Run \"quilt home {} --overwrite\" to reset it anyway.",
                temp_dir.path().display(),
                other.path().display()
            )
        );
        assert_eq!(m.get_home().await?.as_ref(), temp_dir.path());
        Ok(())
    }
}

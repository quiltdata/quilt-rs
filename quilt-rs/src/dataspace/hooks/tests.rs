use super::*;

use std::io::Write;
use std::path::Path;

use tempfile::TempDir;
use test_log::test;

use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::ContractError;
use crate::io::storage::LocalStorage;

type Res = Result<(), Box<dyn std::error::Error>>;

/// The file a hook declared by [`table`] appends its call to.
const RECORD_VAR: &str = "QUILT_TEST_HOOK_RECORD";
/// Set, the hook fails after recording its call.
pub(crate) const FAIL_VAR: &str = "QUILT_TEST_HOOK_FAIL";
/// The text the hook writes to the working folder's metadata file.
pub(crate) const METADATA_VAR: &str = "QUILT_TEST_HOOK_METADATA";

/// The package every flow's tests use.
pub(crate) const PACKAGE: &str = "quilt+s3://b#package=team/raw";

/// A `[hooks.<point>]` table whose script is this test binary running
/// [`child_records_its_call`], which appends its call to `record`, with `env`
/// added. The binary is on every platform, as no shell command is.
pub(crate) fn table(point: HookPoint, record: &Path, env: &[(&str, &str)]) -> String {
    let quote = |text: &str| toml::Value::String(text.to_string()).to_string();
    let exe = std::env::current_exe().expect("the test binary");
    let mut vars = vec![format!(
        "{RECORD_VAR} = {}",
        quote(&record.to_string_lossy())
    )];
    vars.extend(
        env.iter()
            .map(|(name, value)| format!("{name} = {}", quote(value))),
    );
    format!(
        "\n[hooks.{point}]\ncommand = {}\nargs = [\"--exact\", \"dataspace::hooks::tests::child_records_its_call\", \"--ignored\", \"--nocapture\"]\nenv = {{ {} }}\n",
        quote(&exe.to_string_lossy()),
        vars.join(", "),
    )
}

/// One `[hooks.*]` table for each of `points`, each recording to `record`.
pub(crate) fn tables(points: &[HookPoint], record: &Path) -> String {
    points
        .iter()
        .map(|point| table(*point, record, &[]))
        .collect()
}

/// The calls recorded in `record`, in order, each `<point> <revision>
/// <package>`, with `-` for no revision.
pub(crate) fn calls(record: &Path) -> Vec<String> {
    std::fs::read_to_string(record)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A recorded call of `point` for [`PACKAGE`].
pub(crate) fn call(point: HookPoint, revision: Option<&str>) -> String {
    format!("{point} {} {PACKAGE}", revision.unwrap_or("-"))
}

#[test]
#[ignore = "run only as a hook a test declares"]
fn child_records_its_call() -> Res {
    let Ok(record) = std::env::var(RECORD_VAR) else {
        return Ok(());
    };
    let point = std::env::var(HOOK_VAR)?;
    let revision = std::env::var(REVISION_VAR).unwrap_or_else(|_| "-".to_string());
    let package = std::env::var(PACKAGE_VAR)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(record)?;
    writeln!(file, "{point} {revision} {package}")?;
    if let Ok(text) = std::env::var(METADATA_VAR) {
        let folder = std::env::var(FOLDER_VAR)?;
        std::fs::write(Path::new(&folder).join(METADATA_FILE), text)?;
    }
    if std::env::var_os(FAIL_VAR).is_some() {
        return Err("asked to fail".into());
    }
    Ok(())
}

/// A dataspace whose contract reads `team/raw` from bucket `b`, followed by
/// `hooks`.
async fn contract(dir: &TempDir, hooks: &str) -> Result<Contract, ContractError> {
    std::fs::write(
        dir.path().join("data.toml"),
        format!("registry = \"s3://b\"\n\n[packages]\nread = [\"team/raw\"]\n{hooks}"),
    )
    .expect("data.toml");
    Contract::read(&LocalStorage::new(), dir.path(), None).await
}

#[test(tokio::test)]
async fn each_point_takes_one_script_with_its_args_and_env() -> Res {
    let dir = TempDir::new()?;
    let read = contract(
        &dir,
        "[hooks.post-read]\ncommand = \"after-read\"\nargs = [\"--notify\"]\nenv = { STAGE = \"dev\" }\n\
         [hooks.pre-write]\ncommand = \"/opt/stamp\"\n",
    )
    .await?;

    let hooks = read.hooks();
    assert_eq!(
        hooks.get(HookPoint::PostRead),
        Some(&Hook {
            command: "after-read".to_string(),
            args: vec!["--notify".to_string()],
            env: BTreeMap::from([("STAGE".to_string(), "dev".to_string())]),
        })
    );
    assert_eq!(
        hooks.get(HookPoint::PreWrite),
        Some(&Hook {
            command: "/opt/stamp".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        })
    );
    assert_eq!(hooks.get(HookPoint::PreRead), None);
    assert_eq!(hooks.get(HookPoint::PostWrite), None);
    Ok(())
}

#[test(tokio::test)]
async fn a_point_other_than_the_four_and_a_misspelled_key_are_refused() -> Res {
    let dir = TempDir::new()?;
    for hooks in [
        "[hooks.sync]\ncommand = \"tick\"\n",
        "[hooks.post-read]\ncommand = \"a\"\ncommands = [\"b\"]\n",
        "[hooks.post-read]\nargs = [\"no command\"]\n",
    ] {
        let read = contract(&dir, hooks).await;
        assert!(matches!(read, Err(ContractError::Syntax(_))), "{read:?}");
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_hook_runs_in_the_dataspace_with_the_entry_and_revision_in_its_environment() -> Res {
    let dir = TempDir::new()?;
    let record = dir.path().join("calls");
    let read = contract(&dir, &table(HookPoint::PostRead, &record, &[])).await?;
    let entry = &read.entries()[0];

    run(&read, HookPoint::PostRead, entry, Some("abc")).await?;
    // A point the contract leaves out runs nothing.
    run(&read, HookPoint::PreRead, entry, None).await?;

    assert_eq!(calls(&record), [call(HookPoint::PostRead, Some("abc"))]);
    Ok(())
}

#[test(tokio::test)]
async fn a_hook_that_fails_or_cannot_start_is_an_error() -> Res {
    let dir = TempDir::new()?;
    let record = dir.path().join("calls");
    let hooks = format!(
        "{}\n[hooks.pre-write]\ncommand = {}\n",
        table(HookPoint::PreRead, &record, &[(FAIL_VAR, "1")]),
        toml::Value::String(dir.path().join("missing").to_string_lossy().into_owned()),
    );
    let read = contract(&dir, &hooks).await?;
    let entry = &read.entries()[0];

    let failed = run(&read, HookPoint::PreRead, entry, None).await;
    assert!(
        matches!(
            &failed,
            Err(HookError {
                point: HookPoint::PreRead,
                failure: HookFailure::Exit(_),
                ..
            })
        ),
        "{failed:?}"
    );
    let missing = run(&read, HookPoint::PreWrite, entry, None).await;
    assert!(
        matches!(
            &missing,
            Err(HookError {
                point: HookPoint::PreWrite,
                failure: HookFailure::Start(_),
                ..
            })
        ),
        "{missing:?}"
    );
    Ok(())
}

#[test]
fn post_read_follows_only_a_read_that_moved_the_lock() {
    assert_eq!(newer(None, Some("b")), Some("b"));
    assert_eq!(newer(Some("a"), Some("b")), Some("b"));
    assert_eq!(newer(Some("a"), Some("a")), None);
    assert_eq!(newer(Some("a"), None), None);
}

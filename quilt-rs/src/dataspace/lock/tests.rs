use super::*;

use test_log::test;

use crate::dataspace::contract::Registry;
use crate::io::storage::mocks::MockStorage;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

const CONTRACT: &str = "registry = \"s3://quilt-dev\"\n\n[packages]\nread = [\"team/raw\"]\nsync = [{ package = \"team/project\", path = \"src/\" }]\n";

/// A contract with no `sync` entries.
const READ_ONLY: &str = "registry = \"s3://quilt-dev\"\n\n[packages]\nread = [\"team/raw\"]\n";

/// `shasum -a 256` of `CONTRACT`.
const CONTRACT_SHA256: &str = "f6ad0180c93d38ec5189f5c2b6e33c7f4abd9a8dbb5c07caefd093640de017ff";

const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PROJECT_BASE: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const OLDER: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const NEWER: &str = "9999999999999999999999999999999999999999999999999999999999999999";

fn writer() -> Writer {
    Writer {
        host: "ernest-mbp".to_string(),
        user: "ernest".to_string(),
        local_registry: "ernest-mbp".to_string(),
    }
}

fn key(package: &str) -> PackageKey {
    PackageKey {
        registry: Registry::S3("quilt-dev".to_string()),
        package: Namespace::try_from(package).unwrap(),
    }
}

fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

async fn dataspace(
    storage: &MockStorage,
    name: &str,
    contract: &str,
) -> Result<Contract, Box<dyn std::error::Error>> {
    let folder = storage.temp_dir.path().join(name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("data.toml"), contract)?;
    Ok(Contract::read(storage, &folder, None).await?)
}

fn filled_lock(contract: &Contract) -> Lock {
    let mut lock = Lock::new(contract, &writer());
    lock.header.filled = Some(time("2026-11-18T07:45:00Z"));
    lock.packages.insert(
        key("team/raw"),
        LockEntry {
            base: BASE.to_string(),
            commits: Vec::new(),
        },
    );
    lock.packages.insert(
        key("team/project"),
        LockEntry {
            base: PROJECT_BASE.to_string(),
            commits: vec![OLDER.to_string(), NEWER.to_string()],
        },
    );
    lock
}

#[test(tokio::test)]
async fn the_lock_file_has_the_header_then_one_entry_per_package() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let mut lock = filled_lock(&contract);
    lock.write(&storage, &contract, &writer()).await?;

    let written: toml::Table = toml::from_str(&std::fs::read_to_string(
        contract.dataspace().join("data.lock.toml"),
    )?)?;
    let expected: toml::Table = toml::from_str(&format!(
        r#"
[lock]
version = 1
data_toml_hash = "{CONTRACT_SHA256}"
filled = 2026-11-18T07:45:00Z
modified = {modified}
host = "ernest-mbp"
user = "ernest"
dataspace = {dataspace:?}
local_registry = "ernest-mbp"

[packages]
"quilt+s3://quilt-dev#package=team/raw" = "{BASE}"

[packages."quilt+s3://quilt-dev#package=team/project"]
base = "{PROJECT_BASE}"
commits = ["{OLDER}", "{NEWER}"]
"#,
        modified = to_toml_time(lock.header.modified, "modified")?,
        dataspace = contract.dataspace().display().to_string(),
    ))?;
    assert_eq!(written, expected);
    Ok(())
}

#[test(tokio::test)]
async fn a_written_lock_reads_back_as_written() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    assert_eq!(Lock::read(&storage, &contract).await?, None);

    let mut lock = filled_lock(&contract);
    // The file keeps whole seconds, and so does the lock it was written from.
    lock.header.filled = Some(time("2026-11-18T07:45:00.250Z"));
    lock.write(&storage, &contract, &writer()).await?;
    assert_eq!(lock.header.filled, Some(time("2026-11-18T07:45:00Z")));
    assert_eq!(Lock::read(&storage, &contract).await?, Some(lock));
    Ok(())
}

#[test(tokio::test)]
async fn a_write_stamps_the_header_and_keeps_filled() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", READ_ONLY).await?;
    let mut lock = filled_lock(&contract);
    lock.header.writer.host = "elsewhere".to_string();
    lock.header.data_toml_hash = "stale".to_string();
    lock.header.modified = time("2000-01-01T00:00:00Z");

    let before = Utc::now().trunc_subsecs(0);
    lock.write(&storage, &contract, &writer()).await?;
    let after = Utc::now();

    let header = Lock::read(&storage, &contract).await?.unwrap().header;
    assert_eq!(header.writer, writer());
    assert_eq!(header.data_toml_hash, contract.data_toml_hash());
    assert_eq!(header.filled, Some(time("2026-11-18T07:45:00Z")));
    assert!(
        before <= header.modified && header.modified <= after,
        "{header:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_write_under_sync_entries_keeps_the_data_toml_it_was_written_for() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let mut lock = filled_lock(&contract);
    lock.header.data_toml_hash = "stale".to_string();

    lock.write(&storage, &contract, &writer()).await?;

    let header = Lock::read(&storage, &contract).await?.unwrap().header;
    assert_eq!(header.data_toml_hash, "stale");
    Ok(())
}

#[test(tokio::test)]
async fn a_write_replaces_the_lock_whole_and_leaves_no_other_file() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let mut lock = filled_lock(&contract);
    lock.write(&storage, &contract, &writer()).await?;
    lock.packages.remove(&key("team/raw"));
    lock.write(&storage, &contract, &writer()).await?;

    let read = Lock::read(&storage, &contract).await?.unwrap();
    assert_eq!(
        read.packages.keys().collect::<Vec<_>>(),
        [&key("team/project")]
    );
    let mut files: Vec<String> = std::fs::read_dir(contract.dataspace())?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    files.sort();
    assert_eq!(files, ["data.lock.toml", "data.toml"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_written_elsewhere_or_for_another_contract_mismatches() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let lock = filled_lock(&contract);
    assert_eq!(lock.mismatches(&contract, &writer()), []);

    let edited = dataspace(&storage, "rnaseq", &format!("{CONTRACT}# a comment\n")).await?;
    assert_eq!(lock.mismatches(&edited, &writer()), [Mismatch::DataToml]);

    let copied = dataspace(&storage, "copy", CONTRACT).await?;
    assert_eq!(lock.mismatches(&copied, &writer()), [Mismatch::Dataspace]);

    for (other, mismatch) in [
        (
            Writer {
                host: "other".to_string(),
                ..writer()
            },
            Mismatch::Host,
        ),
        (
            Writer {
                user: "other".to_string(),
                ..writer()
            },
            Mismatch::User,
        ),
        (
            Writer {
                local_registry: "other".to_string(),
                ..writer()
            },
            Mismatch::LocalRegistry,
        ),
    ] {
        assert_eq!(lock.mismatches(&contract, &other), [mismatch]);
    }
    Ok(())
}

async fn read_text(text: &str) -> Result<Option<Lock>, LockError> {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await.unwrap();
    std::fs::write(contract.dataspace().join("data.lock.toml"), text).unwrap();
    Lock::read(&storage, &contract).await
}

fn lock_text(version: i64, packages: &str) -> String {
    format!(
        r#"[lock]
version = {version}
data_toml_hash = "{CONTRACT_SHA256}"
modified = 2026-11-18T19:38:00Z
host = "ernest-mbp"
user = "ernest"
dataspace = "/work/rnaseq"
local_registry = "ernest-mbp"

[packages]
{packages}
"#
    )
}

#[test(tokio::test)]
async fn a_time_that_is_not_utc_is_refused() {
    let text = lock_text(1, "").replace("19:38:00Z", "19:38:00+05:00");
    assert!(matches!(
        read_text(&text).await,
        Err(LockError::Time("modified"))
    ));
    let text = lock_text(1, "").replace("19:38:00Z", "19:38:00+00:00");
    let read = read_text(&text).await;
    assert!(
        matches!(&read, Ok(Some(lock)) if lock.header.modified == time("2026-11-18T19:38:00Z")),
        "{read:?}"
    );
}

#[test(tokio::test)]
async fn a_time_toml_cannot_hold_is_an_error_and_the_header_is_kept() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let mut lock = filled_lock(&contract);
    let far = time("9999-12-31T23:59:59Z") + chrono::Duration::days(1);
    lock.header.filled = Some(far);
    lock.header.modified = time("2000-01-01T00:00:00Z");
    let header = lock.header.clone();
    assert!(matches!(
        lock.write(&storage, &contract, &writer()).await,
        Err(LockError::Time("filled"))
    ));
    assert_eq!(lock.header, header);
    assert_eq!(Lock::read(&storage, &contract).await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_the_reader_would_refuse_is_not_written() -> Res {
    let storage = MockStorage::default();
    let contract = dataspace(&storage, "rnaseq", CONTRACT).await?;
    let mut lock = filled_lock(&contract);
    lock.write(&storage, &contract, &writer()).await?;
    let written = Lock::read(&storage, &contract).await?;

    let mut short = lock.clone();
    short.packages.get_mut(&key("team/raw")).unwrap().base = "abc123".to_string();
    let mut unaddressable = lock.clone();
    unaddressable.packages.insert(
        key("team/raw@abc123"),
        LockEntry {
            base: BASE.to_string(),
            commits: Vec::new(),
        },
    );
    for (mut bad, expected) in [(short, "Hash"), (unaddressable, "Key")] {
        let result = bad.write(&storage, &contract, &writer()).await;
        let refused = match &result {
            Err(LockError::Hash { .. }) => "Hash",
            Err(LockError::Key(_)) => "Key",
            _ => "neither",
        };
        assert_eq!(refused, expected, "{result:?}");
        assert_eq!(Lock::read(&storage, &contract).await?, written);
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_of_another_version_is_refused() {
    assert!(matches!(
        read_text(&lock_text(2, "")).await,
        Err(LockError::Version(2))
    ));
}

#[test(tokio::test)]
async fn a_key_is_a_canonical_package_uri() -> Res {
    let read = read_text(&lock_text(
        1,
        &format!(r#""quilt+s3://quilt-dev#package=team/raw" = "{BASE}""#),
    ))
    .await?
    .unwrap();
    assert_eq!(read.packages.keys().collect::<Vec<_>>(), [&key("team/raw")]);
    assert_eq!(read.header.filled, None);

    let local = read_text(&lock_text(
        1,
        &format!(r#""quilt+local://lab-mac#package=me/analysis" = "{BASE}""#),
    ))
    .await?
    .unwrap();
    let local_key = PackageKey {
        registry: Registry::Local("lab-mac".to_string()),
        package: Namespace::try_from("me/analysis")?,
    };
    assert_eq!(local.packages.keys().collect::<Vec<_>>(), [&local_key]);

    for key in [
        "quilt+s3://quilt-dev#package=team/raw:latest",
        &format!("quilt+s3://quilt-dev#package=team/raw@{BASE}"),
        "quilt+s3://quilt-dev#package=team/raw&path=a.csv",
        "quilt+s3://quilt-dev#package=team/raw&catalog=open.quilt.bio",
        "quilt+local://#package=me/analysis",
        "quilt+local://lab-mac#package=me/analysis:latest",
        "team/raw",
    ] {
        let result = read_text(&lock_text(1, &format!(r#""{key}" = "{BASE}""#))).await;
        assert!(
            matches!(&result, Err(LockError::Key(_))),
            "{key}: {result:?}"
        );
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_base_or_commit_is_a_whole_top_hash() {
    for entry in [
        r#""quilt+s3://quilt-dev#package=team/raw" = "abc123""#.to_string(),
        format!(
            r#"[packages."quilt+s3://quilt-dev#package=team/raw"]
base = "{BASE}"
commits = ["abc123"]"#
        ),
    ] {
        let result = read_text(&lock_text(1, &entry)).await;
        assert!(
            matches!(&result, Err(LockError::Hash { package, hash })
                if hash == "abc123" && package == "quilt+s3://quilt-dev#package=team/raw"),
            "{entry}: {result:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_writer_here_is_this_host_and_user() -> Res {
    let run = |command: &str, args: &[&str]| -> std::io::Result<String> {
        let output = std::process::Command::new(command).args(args).output()?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let here = Writer::here("ernest-mbp")?;
    assert_eq!(here.host, run("hostname", &[])?);
    assert_eq!(here.user, run("id", &["-un"])?);
    assert_eq!(here.local_registry, "ernest-mbp");
    Ok(())
}

/// The error for a key does not repeat it, so a user and password written in
/// it stay out of the message.
#[test(tokio::test)]
async fn a_key_error_does_not_repeat_the_key() {
    let key = "quilt+s3://KEY:secret@quilt-dev#package=team/raw";
    let result = read_text(&lock_text(1, &format!(r#""{key}" = "{BASE}""#))).await;
    let Err(err) = result else {
        panic!("{key} should be refused");
    };
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("secret") && !text.contains("KEY"), "{text}");
    }
}

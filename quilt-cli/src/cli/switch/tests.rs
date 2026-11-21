use super::*;

use std::path::Path;

use test_log::test;

use crate::cli::Commands;
use crate::cli::Std;
use crate::cli::init_in;

fn parse(argv: &[&str], on: bool) -> Result<Args, clap::Error> {
    Args::try_parse_switched(std::iter::once("quilt").chain(argv.iter().copied()), on)
}

fn mark() -> PathBuf {
    PathBuf::from(DEFAULT_MARK)
}

/// Since the cutover the switch is on unless `QUILT_DATASPACES` opts out.
#[test]
fn the_switch_is_on_unless_it_opts_out() {
    use std::ffi::OsStr;
    for on in [None, Some(""), Some("1"), Some("yes")] {
        assert!(on_for(on.map(OsStr::new)), "{on:?}");
    }
    for off in ["0", "false", "OFF", " off "] {
        assert!(!on_for(Some(OsStr::new(off))), "{off:?}");
    }
}

/// A cutover that fell back says why, and how to keep the ref path; one
/// that found nothing to do says nothing.
#[test]
fn a_fallback_is_reported_with_the_opt_out() {
    use quilt_rs::domain::import::Cutover;
    use quilt_rs::domain::import::ImportError;

    let mut fell_back = Vec::new();
    report(&Cutover::FellBack(ImportError::InUse), &mut fell_back);
    let mut done = Vec::new();
    report(&Cutover::Done, &mut done);

    let fell_back = String::from_utf8(fell_back).expect("text");
    assert!(fell_back.contains("ref path"), "{fell_back}");
    assert!(fell_back.contains("QUILT_DATASPACES=0"), "{fell_back}");
    assert_eq!(String::from_utf8(done).expect("text"), "");
}

/// A run that falls back is the ref path's, as the switch off is: `--domain`
/// names the application's folder again.
#[test]
fn a_run_that_falls_back_is_parsed_with_the_switch_off() -> Result<(), clap::Error> {
    let off = parse(&["--domain", "/old", "list"], false)?;
    assert!(matches!(
        off.command,
        Commands::List {
            dataspace: None,
            ..
        }
    ));
    assert_eq!(
        Places::new(false, off.domain.clone())
            .ok()
            .map(|places| places.app),
        Some(PathBuf::from("/old"))
    );
    assert!(!settled(), "a test never settles the switch on");
    Ok(())
}

/// Each verb with a dataspace form reads as naming the default dataspace,
/// in whichever shape its form takes the dataspace, and its dataspace-only
/// flags parse with none named.
#[test]
fn with_the_switch_on_a_verb_naming_no_dataspace_names_the_default() -> Result<(), clap::Error> {
    let uri = "quilt+s3://b#package=a/b";
    let installing = parse(&["install", uri, "--access", "read"], true)?;
    assert!(matches!(
        installing.command,
        Commands::Install { dataspace: Some(ref dir), .. } if *dir == mark()
    ));
    let listing = parse(&["list"], true)?;
    assert!(matches!(
        listing.command,
        Commands::List { dataspace: Some(ref dir), .. } if *dir == mark()
    ));
    let reading = parse(&["read"], true)?;
    assert!(matches!(reading.command, Commands::Read { ref dataspace } if *dataspace == mark()));
    let reattaching = parse(&["reattach", uri], true)?;
    assert!(matches!(
        reattaching.command,
        Commands::Reattach { ref dataspace, ref package, .. }
            if *dataspace == mark() && package.to_string() == uri
    ));
    let clearing = parse(&["gc", "--dry-run"], true)?;
    assert!(matches!(
        clearing.command,
        Commands::Gc {
            dataspace: true,
            dry_run: true
        }
    ));
    assert!(matches!(
        parse(&["home"], true)?.command,
        Commands::Home {
            dataspace: true,
            ..
        }
    ));
    // The ref path's own flags are refused, as with `--dataspace` given.
    assert!(parse(&["install", uri, "-n", "a/b"], true).is_err());
    assert!(parse(&["status", "-n", "a/b"], true).is_err());
    Ok(())
}

/// A root option's value that spells a verb is not the verb.
#[test]
fn a_domain_named_like_a_verb_is_not_taken_for_the_verb() -> Result<(), clap::Error> {
    for root in [["--domain", "list"], ["-d", "list"]] {
        let listing = parse(&[root[0], root[1], "list"], true)?;
        assert!(matches!(
            listing.command,
            Commands::List { dataspace: Some(ref dir), .. } if *dir == mark()
        ));
    }
    let listing = parse(&["--domain=list", "list"], true)?;
    assert!(matches!(
        listing.command,
        Commands::List { dataspace: Some(ref dir), .. } if *dir == mark()
    ));
    Ok(())
}

/// Grouped short options: `-vd DIR` and `-vdDIR` take the domain's value,
/// so the verb after them still gets the default dataspace.
#[test]
fn grouped_short_options_are_read_as_clap_reads_them() -> Result<(), clap::Error> {
    for argv in [
        &["-vd", "/tmp/domain", "list"][..],
        &["-vd/tmp/domain", "list"][..],
        &["-vd", "list", "list"][..],
    ] {
        let listing = parse(argv, true)?;
        assert!(
            matches!(
                listing.command,
                Commands::List { dataspace: Some(ref dir), .. } if *dir == mark()
            ),
            "{argv:?}"
        );
    }
    Ok(())
}

/// Off, nothing is defaulted: the parse is today's.
#[test]
fn with_the_switch_off_no_dataspace_is_named() -> Result<(), clap::Error> {
    let uri = "quilt+s3://b#package=a/b";
    assert!(parse(&["install", uri, "--access", "read"], false).is_err());
    assert!(parse(&["read"], false).is_err());
    assert!(parse(&["gc", "--dry-run"], false).is_err());
    let listing = parse(&["list"], false)?;
    assert!(matches!(
        listing.command,
        Commands::List {
            dataspace: None,
            ..
        }
    ));
    assert!(parse(&["status", "-n", "a/b"], false).is_ok());
    Ok(())
}

/// A dataspace named is kept, and a verb with no dataspace form is left as
/// it is.
#[test]
fn with_the_switch_on_a_named_dataspace_and_a_ref_verb_are_kept() -> Result<(), clap::Error> {
    let uri = "quilt+s3://b#package=a/b";
    let listing = parse(&["list", "--dataspace", "ds"], true)?;
    assert!(matches!(
        listing.command,
        Commands::List { dataspace: Some(ref dir), .. } if dir == Path::new("ds")
    ));
    let reattaching = parse(&["reattach", "ds", uri], true)?;
    assert!(matches!(
        reattaching.command,
        Commands::Reattach { ref dataspace, .. } if dataspace == Path::new("ds")
    ));
    let pulling = parse(&["pull", "-n", "a/b"], true)?;
    assert!(matches!(pulling.command, Commands::Pull { .. }));
    Ok(())
}

/// On, `--domain` names the Domain, as `domain::locate` reads it, and the
/// application's folder is the OS's; off, `--domain` names the application's
/// folder and the Domain is the OS's.
#[test]
fn the_switch_moves_the_domain_to_where_domain_locate_finds_it() -> Result<(), Error> {
    let named = tempfile::TempDir::new()?;
    let os_app = super::super::default_domain_dir();
    let os_domain = super::super::dataspace_domain_dir().ok();

    let on = Places::new(true, Some(named.path().to_path_buf()))?;
    assert_eq!(on.domain.as_deref(), Some(named.path()));
    assert_eq!(Some(on.app), os_app);

    let relative = Places::new(true, Some(PathBuf::from("domain-here")))?;
    assert_eq!(
        relative.domain,
        Some(std::env::current_dir()?.join("domain-here"))
    );

    let off = Places::new(false, Some(named.path().to_path_buf()))?;
    assert_eq!(off.app, named.path());
    assert_eq!(off.domain, os_domain);
    Ok(())
}

/// Whether a file or folder named `name` is in `dir`, at any depth.
fn holds(dir: &Path, name: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == name || holds(&entry.path(), name))
    })
}

/// Places in temporary folders.
fn places(dir: &Path) -> Places {
    Places {
        app: dir.join("app"),
        domain: Some(dir.join("domain")),
    }
}

/// Runs `argv` with the switch on in `places`, and its JSON.
async fn run(places: &Places, argv: &[&str]) -> Result<serde_json::Value, Error> {
    let parsed = parse(argv, true).map_err(|err| Error::Test(err.to_string()))?;
    match Box::pin(init_in(parsed, places.clone(), true)).await? {
        Std::Out(out) => Ok(out.to_json()),
        Std::Err(err) => Err(err),
    }
}

/// The default dataspace is the one `home` chose, kept in the application's
/// folder; with none chosen a verb naming none is refused, and one naming a
/// dataspace runs.
#[test(tokio::test)]
async fn the_default_dataspace_is_the_one_home_chose() -> Result<(), Error> {
    let dir = tempfile::TempDir::new()?;
    let places = places(dir.path());
    let space = dir.path().join("space");

    let refused = run(&places, &["list"]).await;
    assert!(
        matches!(refused, Err(Error::NoDefaultDataspace)),
        "{refused:?}"
    );
    assert!(places.default_dataspace().await.is_err());

    let named = dir.path().join("named");
    std::fs::create_dir_all(&named)?;
    std::fs::write(named.join("data.toml"), "")?;
    let listed = run(&places, &["list", "--dataspace", &named.to_string_lossy()]).await?;
    assert_eq!(listed["dataspace"], named.display().to_string());

    let chosen = run(&places, &["home", &space.to_string_lossy()]).await?;
    assert_eq!(chosen["dataspace"], space.display().to_string());
    assert!(places.app.join("default_dataspace.json").is_file());
    assert_eq!(places.default_dataspace().await?, space);
    let listed = run(&places, &["list"]).await?;
    assert_eq!(listed["dataspace"], space.display().to_string());
    Ok(())
}

/// One verb of each family runs on the default dataspace, and the ones that
/// keep commits keep them in the Domain `--domain` would name.
#[test(tokio::test)]
async fn each_family_runs_on_the_default_dataspace_and_the_located_domain() -> Result<(), Error> {
    let dir = tempfile::TempDir::new()?;
    let places = places(dir.path());
    let space = dir.path().join("space");
    let folder = space.join("team").join("new");
    let key = "quilt+local://#package=team/new";
    run(&places, &["home", &space.to_string_lossy()]).await?;
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("a.txt"), b"alpha")?;

    // create
    let created = run(
        &places,
        &["create", key, "--source", &folder.to_string_lossy()],
    )
    .await?;
    assert!(created["commit"].is_string(), "{created}");
    assert!(std::fs::read_to_string(space.join("data.toml"))?.contains("team/new"));

    // commit
    std::fs::write(folder.join("a.txt"), b"beta")?;
    let committed = run(&places, &["commit", "-m", "second", "-n", "team/new"]).await?;
    let commit = committed["commit"]
        .as_str()
        .ok_or(Error::Test(format!("no commit: {committed}")))?;
    assert_eq!(committed["commits"].as_array().map(Vec::len), Some(2));

    // list
    let listed = run(&places, &["list"]).await?;
    assert_eq!(listed["packages"][0]["unpushed"], 2, "{listed}");

    // storage; the located Domain holds the commits.
    let measured = run(&places, &["storage"]).await?;
    assert!(
        measured["total_bytes"].as_u64().unwrap_or(0) > 0,
        "{measured}"
    );
    let domain = places.domain()?;
    assert!(
        holds(domain.root(), commit),
        "{commit} is not in {}",
        domain.root().display()
    );

    // uninstall
    run(&places, &["uninstall", "-n", "team/new"]).await?;
    assert!(!std::fs::read_to_string(space.join("data.toml"))?.contains("team/new"));
    Ok(())
}

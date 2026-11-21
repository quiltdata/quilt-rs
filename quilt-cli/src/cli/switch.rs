//! The switch that makes the dataspace path the default (25u).
//!
//! On, the default since the cutover, a verb that has a dataspace form runs
//! it with no `--dataspace`, on the default dataspace `quilt home --dataspace`
//! chose; `--domain` and `QUILT_DOMAIN` name the Domain, found by
//! [`quilt_rs::domain::locate`]; and the application's folder is the one the
//! OS's local data folder holds. A verb with no dataspace form keeps the ref
//! path. Before the first such verb runs, [`cutover`] imports the packages
//! the ref path kept, once.
//!
//! Off, with [`SWITCH_ENV`] set to `0`, every verb is as it was before the
//! cutover: a dataspace is named with `--dataspace`, `--domain` and
//! `QUILT_DOMAIN` name the application's folder, and the Domain is
//! `bio.quilt.dataspace` in the OS's local data folder. An import that fails
//! leaves the run off too.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::CommandFactory;
use clap::parser::ValueSource;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::home::default_dataspace;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::storage::LocalStorage;

use crate::cli::Args;
use crate::cli::Error;
use crate::cli::dataspace;

/// The variable that turns the switch off, set to `0`, `false` or `off`, for
/// one release after the cutover. Unset, or any other value, leaves it on.
pub const SWITCH_ENV: &str = "QUILT_DATASPACES";

/// Whether this run is on the dataspace path, as [`settle`] set it.
static SETTLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Records whether this run is on the dataspace path: [`from_env`] and
/// [`cutover`] together, as `main` finds it before the command runs.
pub fn settle(on: bool) {
    let _ = SETTLED.set(on);
}

/// Whether this run is on the dataspace path: off where [`settle`] was never
/// called, as in a test that runs a command.
pub fn settled() -> bool {
    SETTLED.get().copied().unwrap_or(false)
}

/// Whether [`SWITCH_ENV`] leaves the switch on, read once per process.
pub fn from_env() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| on_for(std::env::var_os(SWITCH_ENV).as_deref()))
}

/// Whether the switch is on with [`SWITCH_ENV`] set to `value`: on unless
/// it opts out.
pub(super) fn on_for(value: Option<&std::ffi::OsStr>) -> bool {
    !value
        .and_then(std::ffi::OsStr::to_str)
        .map(str::trim)
        .is_some_and(|value| ["0", "false", "off"].contains(&value.to_ascii_lowercase().as_str()))
}

/// The verbs that run on the Domain with the switch on, and so run the
/// cutover's import first.
fn on_the_domain(verb: &str) -> bool {
    FLAG_VERBS.contains(&verb)
        || POSITIONAL_VERBS.contains(&verb)
        || BOOL_VERBS.contains(&verb)
        || ["sync", "local-registry"].contains(&verb)
}

/// Whether this run is on the dataspace path: the switch is on, and, where
/// the verb runs on the Domain, the cutover's import has run, on this start
/// or before. What the import did, or why it fell back, goes to `stderr`;
/// a run that falls back takes the ref path, as the switch off does.
pub async fn cutover(argv: &[OsString], stderr: &mut impl std::io::Write) -> bool {
    if !from_env() {
        return false;
    }
    let Ok(probe) = Args::command()
        .ignore_errors(true)
        .try_get_matches_from(argv)
    else {
        return true;
    };
    let flag = probe.get_one::<PathBuf>("domain").cloned();
    let Ok(places) = Places::new(true, flag) else {
        // The command reports the missing folder itself.
        return true;
    };
    let Some(domain) = places.domain.as_ref().map(DomainRoot::new) else {
        return true;
    };
    let storage = LocalStorage::new();
    let found = if probe.subcommand_name().is_some_and(on_the_domain) {
        Box::pin(quilt_rs::domain::import::cutover(
            &storage,
            &places.app,
            &domain,
        ))
        .await
    } else {
        // A verb with no dataspace form imports nothing, but still refuses a
        // `--domain` that names a folder the ref path keeps packages in.
        match quilt_rs::domain::import::check_domain(&storage, &places.app, &domain).await {
            Ok(()) => return true,
            Err(err) => quilt_rs::domain::import::Cutover::FellBack(err),
        }
    };
    report(&found, stderr);
    found.on()
}

/// Writes what the cutover did to `stderr`, with how to keep the ref path
/// where it fell back.
pub(super) fn report(found: &quilt_rs::domain::import::Cutover, stderr: &mut impl std::io::Write) {
    let Some(report) = found.report() else {
        return;
    };
    let _ = writeln!(stderr, "{report}");
    if !found.on() {
        let _ = writeln!(
            stderr,
            "Set {SWITCH_ENV}=0 to keep the ref path without trying the import again."
        );
    }
}

/// What stands for the default dataspace in the arguments until `init`
/// replaces it with [`Places::default_dataspace`]: clap then checks a verb's
/// flags as it does with a `--dataspace` given.
pub(super) const DEFAULT_MARK: &str = "<default dataspace>";

/// The verbs whose dataspace form takes `--dataspace <DIR>`.
const FLAG_VERBS: [&str; 12] = [
    "install",
    "uninstall",
    "status",
    "list",
    "log",
    "commit",
    "undo-commit",
    "reset",
    "certify",
    "remove-revisions",
    "keep",
    "create",
];

/// The verbs whose dataspace form takes the dataspace as their first
/// argument.
const POSITIONAL_VERBS: [&str; 3] = ["read", "publish", "reattach"];

/// The verbs whose dataspace form is a bare `--dataspace`.
const BOOL_VERBS: [&str; 2] = ["gc", "home"];

/// `argv` with the default dataspace named where the verb has a dataspace
/// form and names none: [`DEFAULT_MARK`] where the form takes a folder, and
/// a bare `--dataspace` where it takes a flag.
///
/// A verb with no dataspace form, an argument list clap cannot read even
/// leniently, and a dataspace already given are left as they are.
pub(super) fn with_default_dataspace(argv: Vec<OsString>) -> Vec<OsString> {
    let probe = Args::command()
        .ignore_errors(true)
        .try_get_matches_from(&argv);
    let Ok(probe) = probe else {
        return argv;
    };
    let Some((verb, matches)) = probe.subcommand() else {
        return argv;
    };
    // Asked only of a verb that has a dataspace form: clap panics on an
    // argument a verb does not have.
    let given = || matches.value_source("dataspace") == Some(ValueSource::CommandLine);
    let inserted: &[&str] = if FLAG_VERBS.contains(&verb) && !given() {
        &["--dataspace", DEFAULT_MARK]
    } else if BOOL_VERBS.contains(&verb) && !given() {
        &["--dataspace"]
    } else if POSITIONAL_VERBS.contains(&verb) && !names_dataspace(matches) {
        &[DEFAULT_MARK]
    } else {
        return argv;
    };
    let Some(at) = verb_index(&argv, verb) else {
        return argv;
    };
    let mut argv = argv;
    let at = at + 1;
    argv.splice(at..at, inserted.iter().map(OsString::from));
    argv
}

/// Where `verb` stands in `argv`: the first argument that is not a root
/// option or that option's value, so `--domain list list` finds the second.
fn verb_index(argv: &[OsString], verb: &str) -> Option<usize> {
    let root = Args::command();
    let mut rest = argv.iter().enumerate().skip(1);
    while let Some((at, token)) = rest.next() {
        let text = token.to_str()?;
        if let Some(long) = text.strip_prefix("--") {
            let takes_value = root
                .get_arguments()
                .find(|opt| opt.get_long() == long.split('=').next())
                .is_some_and(|opt| opt.get_action().takes_values());
            if takes_value && !long.contains('=') {
                rest.next();
            }
        } else if let Some(group) = text.strip_prefix('-').filter(|group| !group.is_empty()) {
            // Short options may be grouped (`-vd DIR`, `-vdDIR`): the first
            // one taking a value takes the rest of the group, or the next
            // argument when it ends the group.
            for (at, short) in group.char_indices() {
                let takes_value = root
                    .get_arguments()
                    .find(|opt| opt.get_short() == Some(short))
                    .is_some_and(|opt| opt.get_action().takes_values());
                if takes_value {
                    if at + short.len_utf8() == group.len() {
                        rest.next();
                    }
                    break;
                }
            }
        } else if !text.starts_with('-') {
            return (text == verb).then_some(at);
        }
    }
    None
}

/// Whether a verb taking the dataspace first was given one. `reattach`'s
/// first argument, where it is a package's URI, is the package: the
/// dataspace was left out.
fn names_dataspace(matches: &clap::ArgMatches) -> bool {
    matches
        .try_get_one::<PathBuf>("dataspace")
        .ok()
        .flatten()
        .is_some_and(|first| {
            first
                .to_str()
                .is_none_or(|first| first.parse::<PackageKey>().is_err())
        })
}

/// Where the dataspace commands find the application's folder, which keeps
/// the credentials and the choice of default dataspace, and the Domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// The application's folder.
    pub app: PathBuf,
    /// The Domain's folder, `None` where the OS has no local data folder.
    pub domain: Option<PathBuf>,
}

impl Places {
    /// The places the switch `on` and `--domain` (or `QUILT_DOMAIN`) name.
    pub fn new(on: bool, domain_flag: Option<PathBuf>) -> Result<Self, Error> {
        if on {
            Ok(Self {
                app: super::default_domain_dir().ok_or(Error::Domain)?,
                domain: quilt_rs::domain::locate(domain_flag.as_deref()),
            })
        } else {
            Ok(Self {
                app: super::get_domain_dir(domain_flag)?,
                domain: super::dataspace_domain_dir().ok(),
            })
        }
    }

    /// The Domain, made where a command makes it.
    pub fn domain(&self) -> Result<DomainRoot, Error> {
        self.domain
            .as_ref()
            .map(DomainRoot::new)
            .ok_or(Error::Domain)
    }

    /// The Domain, where one exists.
    pub fn existing_domain(&self) -> Option<DomainRoot> {
        self.domain.as_deref().and_then(dataspace::existing)
    }

    /// The default dataspace the application chose, or
    /// [`Error::NoDefaultDataspace`].
    pub async fn default_dataspace(&self) -> Result<PathBuf, Error> {
        default_dataspace(&LocalStorage::new(), &self.app)
            .await?
            .ok_or(Error::NoDefaultDataspace)
    }
}

#[cfg(test)]
mod tests;

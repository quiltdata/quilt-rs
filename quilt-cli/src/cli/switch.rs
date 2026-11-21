//! The switch that makes the dataspace path the default (25u).
//!
//! Off, the default, every verb is as it was: a dataspace is named with
//! `--dataspace`, `--domain` and `QUILT_DOMAIN` name the application's folder,
//! and the Domain is `bio.quilt.dataspace` in the OS's local data folder.
//!
//! On, with [`SWITCH_ENV`] set to `1`, a verb that has a dataspace form runs
//! it with no `--dataspace`, on the default dataspace `quilt home --dataspace`
//! chose; `--domain` and `QUILT_DOMAIN` name the Domain, found by
//! [`quilt_rs::domain::locate`]; and the application's folder is the one the
//! OS's local data folder holds. A verb with no dataspace form keeps the ref
//! path.

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

/// The variable that turns the switch on, set to `1`. Unset, or any other
/// value, leaves it off.
pub const SWITCH_ENV: &str = "QUILT_DATASPACES";

/// Whether [`SWITCH_ENV`] turns the switch on, read once per process.
pub fn from_env() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os(SWITCH_ENV).is_some_and(|value| value == "1"))
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

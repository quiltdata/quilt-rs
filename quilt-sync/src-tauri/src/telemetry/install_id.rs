//! The per-install identity: one random value, derived from nothing.
//!
//! It exists so counts of events can become counts of *people* — unique users,
//! retention, a funnel. Deliberately **not** derived from the user: an email
//! hash is reversible for a customer set already known, so it would carry the
//! obligations of personal data while feeling as though it did not, and it would
//! count accounts rather than installs — absent before the first login, so the
//! onboarding head would carry nothing at all.
//!
//! The unit is per-OS-user per machine, which is what "install" means: two
//! accounts on one laptop are two installs, and one person's laptop and desktop
//! are two. Not a session — sessions are the other axis, and the crash SDK
//! supplies those.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::telemetry::prelude::*;

/// The file holding the identity, beside the app's own data.
///
/// A plain file rather than a settings key: it is written once and read on every
/// launch, it must survive a settings-format change, and deleting it is the
/// user-facing way to become a new install — which is also the cheapest shape
/// for an opt-out to reuse later.
const FILE_NAME: &str = "install-id";

/// An install's anonymous identity. Opaque by construction — nothing reads its
/// shape, so the generator's format is free to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallId(String);

impl InstallId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Write `id` so a crash mid-write cannot leave a truncated value.
///
/// Rename within a directory is atomic, so a reader sees either the old file or
/// the whole new one — never half of one. Without this, an interrupted write
/// would leave a partial id that reads as a *different* install forever.
fn persist(path: &Path, id: &str) -> std::io::Result<()> {
    let staging = path.with_extension("tmp");
    std::fs::write(&staging, id)?;
    std::fs::rename(&staging, path)
}

/// The identity for this install, minting one on first run.
///
/// Returns `None` rather than an unpersisted value, and that distinction is the
/// point: an id that is not on disk would be a *new* id next launch, inflating
/// the install count precisely when disks are unhappy — which is the worst
/// moment to also lose confidence in the metric. Better to report no identity
/// for a run than a false one.
///
/// A read that fails for any reason other than absence yields `None` too, and
/// does **not** mint a replacement: overwriting an existing-but-unreadable id
/// would discard a real install's history to satisfy one run.
pub fn load(data_dir: &Path) -> Option<InstallId> {
    let path = path_in(data_dir);

    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let existing = contents.trim();
            if !existing.is_empty() {
                return Some(InstallId(existing.to_string()));
            }
            // An empty file carries no history to protect, so it is treated as
            // absence and replaced.
            debug!("install id file is empty, minting a replacement");
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            warn!("could not read install id, reporting none this run: {err}");
            return None;
        }
    }

    let minted = Uuid::new_v4().to_string();
    if let Err(err) = persist(&path, &minted) {
        warn!("could not persist install id, reporting none this run: {err}");
        return None;
    }
    Some(InstallId(minted))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn mints_and_persists_on_first_run() {
        let dir = TempDir::new().expect("tempdir");

        let first = load(dir.path()).expect("an id on first run");

        assert!(
            path_in(dir.path()).exists(),
            "the id must be on disk, or the next launch invents a new install"
        );
        assert!(
            Uuid::parse_str(first.as_str()).is_ok(),
            "minted ids are random uuids: {}",
            first.as_str()
        );
    }

    /// The property the whole design rests on: the same install reports the same
    /// identity every launch.
    #[test]
    fn is_stable_across_loads() {
        let dir = TempDir::new().expect("tempdir");

        let first = load(dir.path()).expect("first");
        let second = load(dir.path()).expect("second");

        assert_eq!(first, second);
    }

    /// Two installs are two identities — no shared salt, nothing derived from
    /// the machine.
    #[test]
    fn separate_installs_get_separate_identities() {
        let one = TempDir::new().expect("tempdir");
        let other = TempDir::new().expect("tempdir");

        assert_ne!(
            load(one.path()).expect("one"),
            load(other.path()).expect("other")
        );
    }

    /// Surrounding whitespace is tolerated so the file stays hand-editable, and
    /// a trailing newline from an editor does not become part of the identity.
    #[test]
    fn trims_a_hand_edited_file() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(path_in(dir.path()), "  an-id-with-space\n").expect("write");

        assert_eq!(load(dir.path()).expect("id").as_str(), "an-id-with-space");
    }

    /// An empty file carries no history, so it is replaced rather than honoured
    /// as an identity of "".
    #[test]
    fn replaces_an_empty_file() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(path_in(dir.path()), "   ").expect("write");

        let id = load(dir.path()).expect("a replacement id");

        assert!(!id.as_str().is_empty());
        assert!(Uuid::parse_str(id.as_str()).is_ok());
    }

    /// An existing identity is honoured verbatim, not reshaped — the value is
    /// opaque, so a non-uuid on disk is somebody's install, not corruption to
    /// discard.
    #[test]
    fn honours_an_opaque_existing_value() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(path_in(dir.path()), "not-a-uuid-but-still-an-install").expect("write");

        assert_eq!(
            load(dir.path()).expect("id").as_str(),
            "not-a-uuid-but-still-an-install"
        );
    }

    /// An unwritable location yields no identity rather than a fresh one per
    /// launch. A directory standing where the file belongs is the portable way
    /// to make both the read and the write fail.
    #[test]
    fn reports_none_when_it_cannot_persist() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir(path_in(dir.path())).expect("occupy the path with a directory");

        assert_eq!(
            load(dir.path()),
            None,
            "an unpersistable id must be reported as absent, never as a new install"
        );
    }
}

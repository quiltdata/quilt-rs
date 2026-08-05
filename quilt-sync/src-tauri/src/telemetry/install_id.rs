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

/// The file holding the identity, in the app's own data directory beside
/// `publish_settings.json`, `autosync_settings.json`, `logs/` and `.auth/`.
///
/// One concern, one `FILE_NAME`, resolved against `app_local_data_dir` — the
/// convention every other persisted setting here already follows. There is no
/// shared settings framework to join instead.
///
/// Two deliberate departures from those siblings:
///
/// - **A bare value, not JSON.** Wrapping one opaque string in an object buys
///   nothing, and reading it costs support a parse: `cat install_id` answers
///   "which install is this?" directly, which is the question a crash report
///   raises. Deleting the file to become a new install is a clearer affordance
///   for the same reason — and the cheapest shape for an opt-out to reuse later.
/// - **Written atomically, and synchronously.** The settings files write in
///   place; identity cannot afford to, because a torn settings file regenerates
///   from defaults while a torn identity silently becomes a *different* install
///   forever. Synchronous because this is read once during setup, before there is
///   any reason to involve the runtime.
const FILE_NAME: &str = "install_id";

/// An install's anonymous identity. Opaque by construction — nothing reads its
/// shape, so the generator's format is free to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallId(String);

impl InstallId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Write atomically: temp file + rename.
    ///
    /// `create_dir_all` first because this is the **earliest** write into the
    /// data directory — earlier than the logger, which is otherwise what brings
    /// it into being. Without it a genuinely fresh install fails to persist and
    /// its first session goes unattributed, which is the one launch that starts
    /// every funnel.
    ///
    /// Rename within a directory is atomic, so a reader sees either the old file
    /// or the whole new one, never half of one. An interrupted write would
    /// otherwise leave a partial value that reads as a *different* install
    /// forever.
    fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let path = Self::file_path(data_dir);
        let staging = path.with_extension("tmp");
        std::fs::write(&staging, &self.0)?;
        std::fs::rename(&staging, &path)
    }

    /// The identity for this install, minting and persisting one on first run.
    ///
    /// `Option` rather than the `Result`-with-defaults the settings modules
    /// return, because identity has no meaningful default: a fabricated one is
    /// worse than none. Returning `None` rather than an unpersisted value is the
    /// same distinction — an id that is not on disk would be a *new* id next
    /// launch, inflating the install count precisely when disks are unhappy,
    /// which is the worst moment to also lose confidence in the metric.
    ///
    /// A read that fails for any reason other than absence yields `None` too, and
    /// does **not** mint a replacement: overwriting an existing-but-unreadable id
    /// would discard a real install's history to satisfy one run.
    pub fn load(data_dir: &Path) -> Option<Self> {
        match std::fs::read_to_string(Self::file_path(data_dir)) {
            Ok(contents) => {
                let existing = contents.trim();
                if !existing.is_empty() {
                    return Some(Self(existing.to_string()));
                }
                // An empty file carries no history to protect, so it is treated
                // as absence and replaced.
                debug!("install id file is empty, minting a replacement");
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!("could not read install id, reporting none this run: {err}");
                return None;
            }
        }

        let minted = Self(Uuid::new_v4().to_string());
        if let Err(err) = minted.save(data_dir) {
            warn!("could not persist install id, reporting none this run: {err}");
            return None;
        }
        Some(minted)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn mints_and_persists_on_first_run() {
        let dir = TempDir::new().expect("tempdir");

        let first = InstallId::load(dir.path()).expect("an id on first run");

        assert!(
            InstallId::file_path(dir.path()).exists(),
            "the id must be on disk, or the next launch invents a new install"
        );
        assert!(
            Uuid::parse_str(first.as_str()).is_ok(),
            "minted ids are random uuids: {}",
            first.as_str()
        );
    }

    /// A genuinely fresh install: the data directory does not exist yet, because
    /// the identity is loaded before the logger — which is otherwise the thing
    /// that creates it. Without `create_dir_all` this returned `None`, so the
    /// *first* session of every install went unattributed, and the identity only
    /// appeared from the second launch. That is the one launch every funnel starts
    /// from.
    #[test]
    fn mints_on_a_first_run_whose_data_dir_does_not_exist_yet() {
        let parent = TempDir::new().expect("tempdir");
        let data_dir = parent.path().join("not-created-yet");
        assert!(!data_dir.exists(), "the premise of this test");

        let id = InstallId::load(&data_dir).expect("an id on a truly fresh install");

        assert!(InstallId::file_path(&data_dir).exists());
        assert_eq!(
            InstallId::load(&data_dir).as_ref(),
            Some(&id),
            "and it must be the same identity on the next launch"
        );
    }

    /// The property the whole design rests on: the same install reports the same
    /// identity every launch.
    #[test]
    fn is_stable_across_loads() {
        let dir = TempDir::new().expect("tempdir");

        let first = InstallId::load(dir.path()).expect("first");
        let second = InstallId::load(dir.path()).expect("second");

        assert_eq!(first, second);
    }

    /// Two installs are two identities — no shared salt, nothing derived from
    /// the machine.
    #[test]
    fn separate_installs_get_separate_identities() {
        let one = TempDir::new().expect("tempdir");
        let other = TempDir::new().expect("tempdir");

        assert_ne!(
            InstallId::load(one.path()).expect("one"),
            InstallId::load(other.path()).expect("other")
        );
    }

    /// Surrounding whitespace is tolerated so the file stays hand-editable, and
    /// a trailing newline from an editor does not become part of the identity.
    #[test]
    fn trims_a_hand_edited_file() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(InstallId::file_path(dir.path()), "  an-id-with-space\n").expect("write");

        assert_eq!(
            InstallId::load(dir.path()).expect("id").as_str(),
            "an-id-with-space"
        );
    }

    /// An empty file carries no history, so it is replaced rather than honoured
    /// as an identity of "".
    #[test]
    fn replaces_an_empty_file() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(InstallId::file_path(dir.path()), "   ").expect("write");

        let id = InstallId::load(dir.path()).expect("a replacement id");

        assert!(!id.as_str().is_empty());
        assert!(Uuid::parse_str(id.as_str()).is_ok());
    }

    /// An existing identity is honoured verbatim, not reshaped — the value is
    /// opaque, so a non-uuid on disk is somebody's install, not corruption to
    /// discard.
    #[test]
    fn honours_an_opaque_existing_value() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(
            InstallId::file_path(dir.path()),
            "not-a-uuid-but-still-an-install",
        )
        .expect("write");

        assert_eq!(
            InstallId::load(dir.path()).expect("id").as_str(),
            "not-a-uuid-but-still-an-install"
        );
    }

    /// An unwritable location yields no identity rather than a fresh one per
    /// launch. A directory standing where the file belongs is the portable way
    /// to make both the read and the write fail.
    #[test]
    fn reports_none_when_it_cannot_persist() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir(InstallId::file_path(dir.path()))
            .expect("occupy the path with a directory");

        assert_eq!(
            InstallId::load(dir.path()),
            None,
            "an unpersistable id must be reported as absent, never as a new install"
        );
    }
}

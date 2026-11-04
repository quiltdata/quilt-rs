use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::engine::error::Error;

const FILE_NAME: &str = "autosync_settings.json";

/// User-configurable knobs for the background autosync watcher.
///
/// Persisted as `autosync_settings.json` in `app_local_data_dir`. Both
/// directions default to `false` — the loop is opt-in.
///
/// `pull` and `push` are independent because most users want background
/// pulls (cheap, idempotent) without unattended pushes (commits a
/// snapshot of whatever is on disk to the remote). Splitting them lets
/// pull turn on by default in a future release without implicitly
/// opting users into autopush.
///
/// On disk the JSON is flat — `#[serde(flatten)]` projects the nested
/// Rust struct onto the same keys 0.18.0 wrote, so no migration is
/// required.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AutosyncSettings {
    #[serde(flatten)]
    pub pull: PullSettings,
    #[serde(flatten)]
    pub push: PushSettings,
    /// Opt-in close-to-tray. Defaults to `false` so users on
    /// environments without a working tray (stock GNOME) never see
    /// the close button vanish their window. Flat in JSON to keep
    /// the disk shape backwards-compatible.
    #[serde(default)]
    pub close_to_tray: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PullSettings {
    #[serde(default, rename = "pull_enabled")]
    pub enabled: bool,
    /// The one check interval, whatever the window is doing: open, in the
    /// background or closed to the tray. Whole minutes, at least one; see
    /// [`whole_minutes`].
    pub focused_secs: u64,
    /// Not read and not changed: kept, and written back as found, only so
    /// releases 0.22.9 and earlier can read the file.
    #[serde(default = "default_unfocused_secs")]
    pub unfocused_secs: u64,
    /// Not read and not changed: kept, and written back as found, only so
    /// releases 0.22.9 and earlier can read the file.
    #[serde(default = "default_closed_secs")]
    pub closed_secs: u64,
}

impl Default for PullSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            focused_secs: 60,
            unfocused_secs: default_unfocused_secs(),
            closed_secs: default_closed_secs(),
        }
    }
}

const fn default_unfocused_secs() -> u64 {
    120
}

const fn default_closed_secs() -> u64 {
    600
}

/// A check interval as it runs: rounded to the nearest whole minute, and at
/// least one minute. 30 s and 45 s run as 60 s, 90 s as 120 s, 150 s as 180 s.
#[must_use]
pub const fn whole_minutes(secs: u64) -> u64 {
    let minutes = secs.saturating_add(30) / 60;
    if minutes == 0 { 60 } else { minutes * 60 }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PushSettings {
    #[serde(default, rename = "push_enabled")]
    pub enabled: bool,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for PushSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_secs: default_idle_timeout_secs(),
        }
    }
}

const fn default_idle_timeout_secs() -> u64 {
    300
}

impl AutosyncSettings {
    /// These settings with the check interval in whole minutes, the only form
    /// that is loaded, saved or run.
    #[must_use]
    pub fn normalised(mut self) -> Self {
        self.pull.focused_secs = whole_minutes(self.pull.focused_secs);
        self
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    pub async fn load(data_dir: &Path) -> Result<Self, Error> {
        let path = Self::file_path(data_dir);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(serde_json::from_slice::<Self>(&bytes)?.normalised()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::from(err)),
        }
    }

    pub async fn save(&self, data_dir: &Path) -> Result<(), Error> {
        tokio::fs::create_dir_all(data_dir).await?;
        let path = Self::file_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.clone().normalised())?;
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
}

pub type SharedAutosyncSettings = Arc<RwLock<AutosyncSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedAutosyncSettings, Error> {
    let settings = AutosyncSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn nondefault() -> AutosyncSettings {
        AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 300,
                unfocused_secs: 60,
                closed_secs: 900,
            },
            push: PushSettings {
                enabled: true,
                idle_timeout_secs: 45,
            },
            close_to_tray: false,
        }
    }

    #[tokio::test]
    async fn roundtrip_under_new_shape() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = nondefault();
        settings.save(dir.path()).await?;
        assert!(dir.path().join(FILE_NAME).exists());
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_file_returns_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(loaded, AutosyncSettings::default());
        assert!(!loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        Ok(())
    }

    #[tokio::test]
    async fn flat_json_round_trips() -> Result<(), Error> {
        // The disk shape is flat — `#[serde(flatten)]` projects the nested
        // Rust struct onto the same JSON keys the 0.18.0 release wrote.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "pull_enabled": true,
            "push_enabled": false,
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600,
            "idle_timeout_secs": 45
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        assert_eq!(loaded.pull.focused_secs, 60, "30 s runs as one minute");
        assert_eq!(loaded.pull.unfocused_secs, 120);
        assert_eq!(loaded.pull.closed_secs, 600);
        assert_eq!(loaded.push.idle_timeout_secs, 45);
        Ok(())
    }

    #[tokio::test]
    async fn missing_idle_timeout_uses_default() -> Result<(), Error> {
        // 0.18.0 JSON shape: no `idle_timeout_secs` key.
        // `#[serde(default = "default_idle_timeout_secs")]` must fill in 300.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "pull_enabled": true,
            "push_enabled": true,
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(
            loaded.push.idle_timeout_secs, 300,
            "0.18.0 files (no idle_timeout_secs) must load with the 300s default"
        );
        // Other fields preserved verbatim.
        assert!(loaded.pull.enabled);
        assert!(loaded.push.enabled);
        assert_eq!(loaded.pull.focused_secs, 60);
        assert_eq!(loaded.pull.unfocused_secs, 120);
        assert_eq!(loaded.pull.closed_secs, 600);
        Ok(())
    }

    #[tokio::test]
    async fn missing_enabled_keys_default_to_false() -> Result<(), Error> {
        // Today's behavior: a JSON file missing `pull_enabled` / `push_enabled`
        // loads as `false` rather than erroring. `#[serde(default)]` on the
        // two `enabled` fields preserves that.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600,
            "idle_timeout_secs": 30
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(!loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        Ok(())
    }

    #[tokio::test]
    async fn close_to_tray_defaults_to_false_and_round_trips() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(
            !loaded.close_to_tray,
            "missing file must yield close_to_tray = false",
        );

        let mut settings = nondefault();
        settings.close_to_tray = true;
        settings.save(dir.path()).await?;
        let reloaded = AutosyncSettings::load(dir.path()).await?;
        assert!(reloaded.close_to_tray);
        assert_eq!(reloaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_close_to_tray_key_loads_as_false() -> Result<(), Error> {
        // 0.18.1 JSON shape: no close_to_tray key. `#[serde(default)]`
        // must fill in false so existing files load unchanged.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "pull_enabled": true,
            "push_enabled": false,
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600,
            "idle_timeout_secs": 45
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(!loaded.close_to_tray);
        Ok(())
    }

    /// The table the owner settled: nearest whole minute, at least one.
    const NORMALISED: [(u64, u64); 9] = [
        (0, 60),
        (1, 60),
        (30, 60),
        (45, 60),
        (89, 60),
        (90, 120),
        (150, 180),
        (300, 300),
        (u64::MAX, u64::MAX / 60 * 60),
    ];

    #[test]
    fn an_interval_runs_in_whole_minutes_at_least_one() {
        for (stored, runs) in NORMALISED {
            assert_eq!(whole_minutes(stored), runs, "{stored} s");
        }
    }

    #[test]
    fn the_default_interval_is_one_minute() {
        let pull = AutosyncSettings::default().pull;
        assert_eq!(pull.focused_secs, 60);
        assert_eq!((pull.unfocused_secs, pull.closed_secs), (120, 600));
    }

    fn file_with_interval(secs: u64) -> String {
        format!(
            r#"{{"pull_enabled": true, "push_enabled": false, "focused_secs": {secs},
                "unfocused_secs": 120, "closed_secs": 600, "idle_timeout_secs": 45}}"#
        )
    }

    #[tokio::test]
    async fn load_normalises_the_interval() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        for (stored, runs) in NORMALISED {
            tokio::fs::write(dir.path().join(FILE_NAME), file_with_interval(stored)).await?;
            let loaded = AutosyncSettings::load(dir.path()).await?;
            assert_eq!(loaded.pull.focused_secs, runs, "{stored} s");
        }
        Ok(())
    }

    #[tokio::test]
    async fn save_normalises_the_interval() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        for (stored, runs) in NORMALISED {
            let mut settings = AutosyncSettings::default();
            settings.pull.focused_secs = stored;
            settings.save(dir.path()).await?;
            let bytes = tokio::fs::read(dir.path().join(FILE_NAME)).await?;
            let on_disk: serde_json::Value = serde_json::from_slice(&bytes)?;
            assert_eq!(on_disk["focused_secs"], runs, "{stored} s");
        }
        Ok(())
    }

    /// The two retired intervals are not this version's: a load and a save
    /// write them back exactly as found, odd values included.
    #[tokio::test]
    async fn load_then_save_keeps_the_retired_intervals() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        tokio::fs::write(
            &path,
            br#"{"pull_enabled": true, "push_enabled": false, "focused_secs": 90,
                "unfocused_secs": 7, "closed_secs": 1234, "idle_timeout_secs": 45}"#,
        )
        .await?;
        AutosyncSettings::load(dir.path())
            .await?
            .save(dir.path())
            .await?;
        let on_disk: serde_json::Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        assert_eq!(on_disk["unfocused_secs"], 7);
        assert_eq!(on_disk["closed_secs"], 1234);
        assert_eq!(on_disk["focused_secs"], 120);
        Ok(())
    }

    #[tokio::test]
    async fn a_file_without_the_retired_intervals_gets_the_old_defaults() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(
            dir.path().join(FILE_NAME),
            br#"{"pull_enabled": true, "focused_secs": 60}"#,
        )
        .await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(
            (loaded.pull.unfocused_secs, loaded.pull.closed_secs),
            (120, 600)
        );
        Ok(())
    }

    /// The settings shape release 0.22.9 reads, copied as it shipped: every
    /// interval required.
    #[derive(Deserialize)]
    struct Settings0229 {
        #[serde(flatten)]
        pull: Pull0229,
        #[serde(flatten)]
        push: Push0229,
        #[serde(default)]
        close_to_tray: bool,
    }

    #[derive(Deserialize)]
    struct Pull0229 {
        #[serde(default, rename = "pull_enabled")]
        enabled: bool,
        focused_secs: u64,
        unfocused_secs: u64,
        closed_secs: u64,
    }

    #[derive(Deserialize)]
    struct Push0229 {
        #[serde(default, rename = "push_enabled")]
        enabled: bool,
        #[serde(default = "default_idle_timeout_secs")]
        idle_timeout_secs: u64,
    }

    /// A downgrade reads what this version writes, with the same choices.
    #[tokio::test]
    async fn quiltsync_0_22_9_reads_a_file_this_version_writes() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        for settings in [AutosyncSettings::default(), nondefault()] {
            let mut settings = settings;
            settings.close_to_tray = !settings.close_to_tray;
            settings.save(dir.path()).await?;
            let bytes = tokio::fs::read(dir.path().join(FILE_NAME)).await?;
            let old: Settings0229 = serde_json::from_slice(&bytes)?;
            assert_eq!(old.pull.enabled, settings.pull.enabled);
            assert_eq!(old.push.enabled, settings.push.enabled);
            assert_eq!(old.close_to_tray, settings.close_to_tray);
            assert_eq!(old.pull.focused_secs, settings.pull.focused_secs);
            assert_eq!(old.pull.unfocused_secs, settings.pull.unfocused_secs);
            assert_eq!(old.pull.closed_secs, settings.pull.closed_secs);
            assert_eq!(old.push.idle_timeout_secs, settings.push.idle_timeout_secs);
        }
        Ok(())
    }
}

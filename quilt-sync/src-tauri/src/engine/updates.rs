//! Whether a newer `QuiltSync` is out, read from the update feed over HTTP.
//!
//! The same in both modes: the desktop app installs what it finds through the
//! desktop's updater; headless can't install, and points to the download.

use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::engine::telemetry::prelude::*;

/// The feed the desktop's updater reads too (`tauri.conf.json`).
const FEED: &str = "https://www.quilt.bio/hubfs/latest.json";

/// How long a check's answer stands before the next asks the feed again.
#[allow(clippy::duration_suboptimal_units)]
const FRESH: Duration = Duration::from_secs(6 * 60 * 60);

/// How long a failed read stands, so an outage costs one request, not one per
/// page.
#[allow(clippy::duration_suboptimal_units)]
const RETRY: Duration = Duration::from_secs(10 * 60);

/// A newer version, and where to download it for this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// The installer to download by hand, or the release page when the feed
    /// has no build for this machine.
    pub url: String,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    #[serde(default)]
    platforms: std::collections::HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
}

pub struct Updates {
    current: Version,
    feed: &'static str,
    /// When to ask the feed again, and the last version it offered.
    last: Mutex<Option<(Instant, Option<UpdateInfo>)>>,
}

impl Updates {
    pub fn new(current: Version) -> Self {
        Self {
            current,
            feed: FEED,
            last: Mutex::new(None),
        }
    }

    /// The last check's answer, without asking: `None` until a check is in.
    pub fn known(&self) -> Option<UpdateInfo> {
        self.last
            .try_lock()
            .ok()
            .and_then(|last| last.as_ref().and_then(|(_, found)| found.clone()))
    }

    /// A newer version, asking the feed when the last answer is stale.
    ///
    /// # Errors
    ///
    /// When the feed can't be read; checks until [`RETRY`] has passed answer
    /// with what was known before.
    pub async fn check(&self) -> Result<Option<UpdateInfo>, String> {
        let mut last = self.last.lock().await;
        if let Some((again, found)) = last.as_ref()
            && Instant::now() < *again
        {
            return Ok(found.clone());
        }
        match self.read_feed().await {
            Ok(found) => {
                *last = Some((Instant::now() + FRESH, found.clone()));
                Ok(found)
            }
            Err(err) => {
                let known = last.take().and_then(|(_, found)| found);
                *last = Some((Instant::now() + RETRY, known));
                Err(err)
            }
        }
    }

    async fn read_feed(&self) -> Result<Option<UpdateInfo>, String> {
        let feed: Feed = reqwest::Client::new()
            .get(self.feed)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(newer(&self.current, &feed))
    }
}

/// What `feed` offers over `current`, with the download for this machine.
fn newer(current: &Version, feed: &Feed) -> Option<UpdateInfo> {
    let offered = Version::parse(feed.version.trim_start_matches('v'))
        .inspect_err(|e| warn!("update feed has an unreadable version: {e}"))
        .ok()?;
    if offered <= *current {
        return None;
    }
    let url = feed.platforms.get(&platform_key()).map_or_else(
        || format!("https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v{offered}"),
        |platform| installer(&platform.url),
    );
    Some(UpdateInfo {
        version: offered.to_string(),
        url,
    })
}

/// The installer for an updater download: on macOS the feed gives the
/// updater's `.app.tar.gz`, and each release puts the `.dmg` beside it.
fn installer(url: &str) -> String {
    url.strip_suffix(".app.tar.gz")
        .map_or_else(|| url.to_owned(), |stem| format!("{stem}.dmg"))
}

/// This machine as the feed names it, e.g. `darwin-aarch64`.
fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(version: &str) -> Feed {
        Feed {
            version: version.to_owned(),
            platforms: [(
                platform_key(),
                Platform {
                    url: "https://example.test/QuiltSync.pkg".to_owned(),
                },
            )]
            .into(),
        }
    }

    #[test]
    fn only_a_newer_version_is_an_update() {
        let current = Version::new(0, 26, 1);
        assert_eq!(newer(&current, &feed("0.26.1")), None);
        assert_eq!(newer(&current, &feed("0.26.0")), None);
        assert_eq!(
            newer(&current, &feed("v0.27.0")),
            Some(UpdateInfo {
                version: "0.27.0".to_owned(),
                url: "https://example.test/QuiltSync.pkg".to_owned(),
            })
        );
    }

    #[test]
    fn a_feed_without_this_machine_points_to_the_release_page() {
        let mut feed = feed("1.0.0");
        feed.platforms.clear();
        let found = newer(&Version::new(0, 1, 0), &feed).unwrap();
        assert_eq!(
            found.url,
            "https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v1.0.0"
        );
    }

    #[test]
    fn a_macos_updater_download_becomes_the_dmg() {
        let base = "https://www.quilt.bio/hubfs/quiltsync/QuiltSync/v0.26.1/QuiltSync_0.26.1";
        assert_eq!(
            installer(&format!("{base}_aarch64.app.tar.gz")),
            format!("{base}_aarch64.dmg")
        );
        assert_eq!(
            installer(&format!("{base}_x64_en-US.msi")),
            format!("{base}_x64_en-US.msi")
        );
    }

    #[tokio::test]
    async fn a_failed_read_is_not_retried_at_once() {
        let updates = Updates {
            feed: "http://127.0.0.1:1/latest.json",
            ..Updates::new(Version::new(0, 1, 0))
        };
        assert!(updates.check().await.is_err());
        assert_eq!(updates.check().await, Ok(None), "the failure stands");
    }

    /// Both read the same feed; a move of one must move the other.
    #[test]
    fn the_feed_is_the_desktop_updaters() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["plugins"]["updater"]["endpoints"][0], FEED);
    }
}

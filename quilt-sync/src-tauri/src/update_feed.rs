//! The update check without Tauri: read the updater feed and compare its
//! version with ours. Installing stays desktop-only.

use semver::Version;
use serde::Deserialize;

use crate::commands::UpdateInfo;
use crate::telemetry::prelude::*;

/// `plugins.updater.endpoints` in `tauri.conf.json`.
const FEED_URL: &str = "https://www.quilt.bio/hubfs/latest.json";

#[derive(Deserialize)]
struct Feed {
    version: String,
}

/// The feed's version, when it is newer than `ours`.
fn newer(feed: &str, ours: &Version) -> Result<Option<UpdateInfo>, String> {
    let feed: Feed = serde_json::from_str(feed).map_err(|e| e.to_string())?;
    let theirs = Version::parse(feed.version.trim_start_matches('v')).map_err(|e| e.to_string())?;
    Ok((theirs > *ours).then(|| UpdateInfo {
        version: theirs.to_string(),
    }))
}

/// What the desktop `check_for_update` answers, read from the feed.
pub async fn check(ours: &Version) -> Result<Option<UpdateInfo>, String> {
    let fetched = async {
        reqwest::get(FEED_URL)
            .await?
            .error_for_status()?
            .text()
            .await
    }
    .await
    .map_err(|e| e.to_string());
    let checked = fetched.and_then(|text| newer(&text, ours));
    if let Err(e) = &checked {
        warn!("update check failed: {e}");
    }
    checked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(feed: &str, ours: &str) -> Option<String> {
        newer(feed, &Version::parse(ours).unwrap())
            .unwrap()
            .map(|i| i.version)
    }

    #[test]
    fn only_a_newer_version_is_an_update() {
        let feed = r#"{"version": "0.23.2", "notes": "", "platforms": {}}"#;
        assert_eq!(version(feed, "0.23.1"), Some("0.23.2".into()));
        assert_eq!(version(feed, "0.23.2"), None);
        assert_eq!(version(feed, "0.23.3-dev"), None);
        assert_eq!(
            version(r#"{"version": "v1.0.0"}"#, "0.23.2"),
            Some("1.0.0".into())
        );
        assert!(newer("{}", &Version::new(0, 1, 0)).is_err());
    }
}

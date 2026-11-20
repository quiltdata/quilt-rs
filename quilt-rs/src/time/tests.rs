//! The text of every time quilt-rs writes, and the older shapes it still reads.

use serde_json::json;

use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::Tokens;
use crate::lineage::CommitState;
use crate::lineage::PathState;

type Time = chrono::DateTime<chrono::Utc>;

fn at(text: &str) -> Time {
    text.parse().expect("a time")
}

/// What a written time reads as, and the text it is written as: sub-second
/// digits come in groups of three, none when the fraction is zero.
const WRITTEN: [(&str, &str); 8] = [
    ("2026-11-12T20:00:00Z", "2026-11-12T20:00:00Z"),
    ("2026-11-12T20:00:00.000Z", "2026-11-12T20:00:00Z"),
    ("2025-01-16T12:50:20.534Z", "2025-01-16T12:50:20.534Z"),
    ("2025-01-16T12:50:20.1Z", "2025-01-16T12:50:20.100Z"),
    ("2025-01-16T12:50:20.5341Z", "2025-01-16T12:50:20.534100Z"),
    (
        "2025-01-16T12:50:20.5341007Z",
        "2025-01-16T12:50:20.534100700Z",
    ),
    (
        "2025-01-16T12:50:20.000000001Z",
        "2025-01-16T12:50:20.000000001Z",
    ),
    ("1969-12-31T23:59:59.5Z", "1969-12-31T23:59:59.500Z"),
];

/// Shapes an older file or another writer may hold, each the same instant as
/// `2026-11-12T20:00:00Z` or, with a fraction, `…:00.500Z`.
const READ: [(&str, &str); 7] = [
    ("2026-11-12T21:00:00+01:00", "2026-11-12T20:00:00Z"),
    ("2026-11-12T19:30:00-00:30", "2026-11-12T20:00:00Z"),
    ("2026-11-12T20:00:00-00:00", "2026-11-12T20:00:00Z"),
    ("2026-11-12 20:00:00Z", "2026-11-12T20:00:00Z"),
    ("2026-11-12t20:00:00z", "2026-11-12T20:00:00Z"),
    ("2026-11-12T20:00:00.500000000Z", "2026-11-12T20:00:00.500Z"),
    ("2026-11-12T18:30:00.5-01:30", "2026-11-12T20:00:00.500Z"),
];

#[test]
fn a_tracked_path_writes_its_time_in_the_json_shape() {
    for (time, text) in WRITTEN {
        let state = PathState {
            timestamp: at(time),
            hash: None,
        };
        let written = serde_json::to_value(&state).unwrap();
        assert_eq!(written, json!({ "timestamp": text }), "{time}");
        let back: PathState = serde_json::from_value(written).unwrap();
        assert_eq!(back, state, "{time}");
    }
}

#[test]
fn a_tracked_path_reads_older_shapes() {
    for (text, same) in READ {
        let state: PathState = serde_json::from_value(json!({ "timestamp": text })).unwrap();
        assert_eq!(state.timestamp, at(same), "{text}");
    }
}

#[test]
fn a_commit_writes_its_time_in_the_json_shape() {
    for (time, text) in WRITTEN {
        let commit = CommitState {
            timestamp: at(time),
            hash: "h".to_string(),
            prev_hashes: Vec::new(),
        };
        let written = serde_json::to_string(&commit).unwrap();
        assert_eq!(
            written,
            format!(r#"{{"timestamp":"{text}","hash":"h","prev_hashes":[]}}"#),
        );
        let back: CommitState = serde_json::from_str(&written).unwrap();
        assert_eq!(back, commit);
    }
    for (text, same) in READ {
        let commit: CommitState =
            serde_json::from_value(json!({ "timestamp": text, "hash": "h" })).unwrap();
        assert_eq!(commit.timestamp, at(same), "{text}");
    }
}

#[test]
fn the_auth_files_write_expiry_in_the_json_shape() {
    for (time, text) in WRITTEN {
        let tokens = Tokens {
            access_token: "a".to_string(),
            refresh_token: "r".to_string(),
            expires_at: at(time),
        };
        let written = serde_json::to_string(&tokens).unwrap();
        assert_eq!(
            written,
            format!(r#"{{"access_token":"a","refresh_token":"r","expires_at":"{text}"}}"#),
        );
        let back: Tokens = serde_json::from_str(&written).unwrap();
        assert_eq!(back.expires_at, tokens.expires_at);

        let credentials = Credentials {
            access_key: "k".to_string(),
            secret_key: "s".to_string(),
            token: "t".to_string(),
            expires_at: at(time),
        };
        let written = serde_json::to_string(&credentials).unwrap();
        assert_eq!(
            written,
            format!(r#"{{"access_key":"k","secret_key":"s","token":"t","expires_at":"{text}"}}"#),
        );
        let back: Credentials = serde_json::from_str(&written).unwrap();
        assert_eq!(back.expires_at, credentials.expires_at);
    }
    for (text, same) in READ {
        let tokens: Tokens = serde_json::from_value(json!({
            "access_token": "a",
            "refresh_token": "r",
            "expires_at": text,
        }))
        .unwrap();
        assert_eq!(tokens.expires_at, at(same), "{text}");
    }
}

#[test]
fn a_time_with_no_offset_is_refused() {
    let read = serde_json::from_value::<PathState>(json!({ "timestamp": "2026-11-12T20:00:00" }));
    assert!(read.is_err());
}

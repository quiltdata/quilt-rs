use serde::Serialize;

/// How many recent changelog entries to display.
const MAX_ENTRIES: usize = 5;

static CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// A single version entry from CHANGELOG.md.
#[derive(Serialize)]
pub struct ChangelogEntry {
    /// Version string, e.g. "v0.14.6-alpha2".
    pub version: String,
    /// Release date, e.g. "2026-04-02".
    pub date: String,
    /// Raw body text (section headers + bullet points).
    pub body: String,
}

/// Parse the latest entries from the embedded CHANGELOG.md.
pub fn latest_entries() -> Vec<ChangelogEntry> {
    let mut entries = Vec::new();

    // Split on version headers: ## [vX.Y.Z...] - YYYY-MM-DD
    for chunk in CHANGELOG.split("\n## [") {
        if entries.len() >= MAX_ENTRIES {
            break;
        }

        // Each chunk starts with: vX.Y.Z...] - YYYY-MM-DD\n...body...
        let Some((header, body)) = chunk.split_once('\n') else {
            continue;
        };

        // Parse "vX.Y.Z-alphaN] - 2026-04-02"
        let Some((version, rest)) = header.split_once(']') else {
            continue;
        };

        let date = rest.trim().trim_start_matches("- ").trim().to_string();
        if date.is_empty() {
            continue;
        }

        let body = body
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();

        entries.push(ChangelogEntry {
            version: version.to_string(),
            date,
            body,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_entries() {
        let entries = latest_entries();
        assert!(!entries.is_empty());
        assert!(entries.len() <= MAX_ENTRIES);

        let first = &entries[0];
        assert!(first.version.starts_with('v'), "version: {}", first.version);
        assert!(!first.date.is_empty());
        assert!(!first.body.is_empty());
    }

    #[test]
    fn test_limits_entries() {
        let entries = latest_entries();
        assert!(entries.len() <= MAX_ENTRIES);
    }
}

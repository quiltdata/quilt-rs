use std::path::PathBuf;

use chrono::Local;

use crate::quilt::lineage::{Change, ChangeSet};
use crate::quilt::uri::Namespace;

/// Generates a concise, human-readable commit message string from the set of changed files.
///
/// For three or fewer total changes, individual file names are listed.
/// For larger changesets, counts are used instead.
pub fn generate(changes: &ChangeSet) -> String {
    let added: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Added(_)))
        .map(|(p, _)| p)
        .collect();
    let modified: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Modified(_)))
        .map(|(p, _)| p)
        .collect();
    let removed: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Removed(_)))
        .map(|(p, _)| p)
        .collect();

    let total = changes.len();
    if total == 0 {
        return String::new();
    }

    let mut parts = Vec::new();
    if total <= 3 {
        if !added.is_empty() {
            parts.push(format!("Add {}", file_names(&added)));
        }
        if !modified.is_empty() {
            parts.push(format!("Update {}", file_names(&modified)));
        }
        if !removed.is_empty() {
            parts.push(format!("Remove {}", file_names(&removed)));
        }
    } else {
        if !added.is_empty() {
            parts.push(change_count(added.len(), "Add"));
        }
        if !modified.is_empty() {
            parts.push(change_count(modified.len(), "Update"));
        }
        if !removed.is_empty() {
            parts.push(change_count(removed.len(), "Remove"));
        }
    }
    parts.join(", ")
}

fn file_names(paths: &[&PathBuf]) -> String {
    paths
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn change_count(n: usize, verb: &str) -> String {
    if n == 1 {
        format!("{verb} 1 file")
    } else {
        format!("{verb} {n} files")
    }
}

/// Context available to a Publish message template.
///
/// Keeps the `render_publish_message` helper UI-agnostic: the caller supplies
/// the already-computed `changes` summary and the target `namespace`, and
/// `{date}`/`{time}`/`{datetime}` are filled from the local clock at render
/// time.
pub struct PublishMessageContext<'a> {
    pub namespace: &'a Namespace,
    pub changes_summary: String,
}

/// Render a user-configured message template for Publish.
///
/// Supported placeholders: `{date}` (`YYYY-MM-DD`), `{time}` (`HH:MM`),
/// `{datetime}` (`YYYY-MM-DD HH:MM`), `{namespace}`, `{changes}`. Unknown
/// placeholders pass through verbatim so typos are visible in the preview.
/// An empty (or whitespace-only) template falls back to the auto-generated
/// summary from [`generate`].
pub fn render_publish_message(template: &str, ctx: &PublishMessageContext<'_>) -> String {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return ctx.changes_summary.clone();
    }
    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M").to_string();
    let datetime = format!("{date} {time}");
    template
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{datetime}", &datetime)
        .replace("{namespace}", &ctx.namespace.to_string())
        .replace("{changes}", &ctx.changes_summary)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::quilt::manifest::ManifestRow;

    fn make_changes(added: &[&str], modified: &[&str], removed: &[&str]) -> ChangeSet {
        let mut changes = BTreeMap::new();
        for name in added {
            changes.insert(PathBuf::from(name), Change::Added(ManifestRow::default()));
        }
        for name in modified {
            changes.insert(
                PathBuf::from(name),
                Change::Modified(ManifestRow::default()),
            );
        }
        for name in removed {
            changes.insert(PathBuf::from(name), Change::Removed(ManifestRow::default()));
        }
        changes
    }

    #[test]
    fn test_empty() {
        assert_eq!(generate(&BTreeMap::new()), "");
    }

    #[test]
    fn test_single_add() {
        let changes = make_changes(&["results.csv"], &[], &[]);
        assert_eq!(generate(&changes), "Add results.csv");
    }

    #[test]
    fn test_single_modify() {
        let changes = make_changes(&[], &["data.parquet"], &[]);
        assert_eq!(generate(&changes), "Update data.parquet");
    }

    #[test]
    fn test_single_remove() {
        let changes = make_changes(&[], &[], &["old.csv"]);
        assert_eq!(generate(&changes), "Remove old.csv");
    }

    #[test]
    fn test_mixed_few() {
        let changes = make_changes(&["results.csv"], &[], &["old.csv"]);
        assert_eq!(generate(&changes), "Add results.csv, Remove old.csv");
    }

    #[test]
    fn test_three_files() {
        let changes = make_changes(&["a.csv", "b.csv"], &["c.csv"], &[]);
        assert_eq!(generate(&changes), "Add a.csv, b.csv, Update c.csv");
    }

    #[test]
    fn test_many_adds() {
        let names: Vec<String> = (1..=5).map(|i| format!("file{i}.csv")).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let changes = make_changes(&name_refs, &[], &[]);
        assert_eq!(generate(&changes), "Add 5 files");
    }

    #[test]
    fn test_many_mixed() {
        let added: Vec<String> = (1..=3).map(|i| format!("add{i}.csv")).collect();
        let modified: Vec<String> = (1..=2).map(|i| format!("mod{i}.csv")).collect();
        let removed = ["old.csv".to_string()];
        let changes = make_changes(
            &added.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &modified.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &removed.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        assert_eq!(
            generate(&changes),
            "Add 3 files, Update 2 files, Remove 1 file"
        );
    }

    #[test]
    fn test_uses_filename_not_full_path() {
        let changes = make_changes(&["subdir/data/results.csv"], &[], &[]);
        assert_eq!(generate(&changes), "Add results.csv");
    }

    fn ctx(summary: &str) -> (Namespace, String) {
        (Namespace::from(("user", "pkg")), summary.to_string())
    }

    #[test]
    fn render_empty_template_falls_back_to_summary() {
        let (ns, summary) = ctx("Add a, b");
        let rendered = render_publish_message(
            "",
            &PublishMessageContext {
                namespace: &ns,
                changes_summary: summary,
            },
        );
        assert_eq!(rendered, "Add a, b");
    }

    #[test]
    fn render_whitespace_template_falls_back_to_summary() {
        let (ns, summary) = ctx("Add a");
        let rendered = render_publish_message(
            "   \t\n",
            &PublishMessageContext {
                namespace: &ns,
                changes_summary: summary,
            },
        );
        assert_eq!(rendered, "Add a");
    }

    #[test]
    fn render_substitutes_namespace_and_changes() {
        let (ns, summary) = ctx("Add data.csv");
        let rendered = render_publish_message(
            "Publish {namespace}: {changes}",
            &PublishMessageContext {
                namespace: &ns,
                changes_summary: summary,
            },
        );
        assert_eq!(rendered, "Publish user/pkg: Add data.csv");
    }

    #[test]
    fn render_leaves_unknown_placeholders_intact() {
        let (ns, summary) = ctx("Update c.csv");
        let rendered = render_publish_message(
            "Release {dat} {changes} by {user}",
            &PublishMessageContext {
                namespace: &ns,
                changes_summary: summary,
            },
        );
        assert_eq!(rendered, "Release {dat} Update c.csv by {user}");
    }

    #[test]
    fn render_fills_date_time_datetime() {
        let (ns, summary) = ctx("Add f.txt");
        let rendered = render_publish_message(
            "{date} {time} -> {datetime}",
            &PublishMessageContext {
                namespace: &ns,
                changes_summary: summary,
            },
        );
        let parts: Vec<&str> = rendered.split(" -> ").collect();
        assert_eq!(parts.len(), 2);
        // Shape only; values depend on clock.
        let date_time = parts[0];
        let dt = parts[1];
        assert_eq!(date_time.len(), "2026-04-21 12:34".len());
        assert_eq!(dt.len(), "2026-04-21 12:34".len());
    }
}

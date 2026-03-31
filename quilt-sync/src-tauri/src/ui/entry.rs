use std::path::PathBuf;
use std::string::String;

use askama::Template;
use rust_i18n::t;

use crate::quilt;
use crate::quilt::lineage::Change;
use crate::ui::btn;
use crate::ui::Icon;

#[derive(Debug)]
pub enum EntryStatus {
    Added,
    Deleted,
    Modified,
    Pristine,
    Remote,
}

impl From<&Change> for EntryStatus {
    fn from(change: &Change) -> Self {
        match change {
            Change::Modified(_) => EntryStatus::Modified,
            Change::Added(_) => EntryStatus::Added,
            Change::Removed(_) => EntryStatus::Deleted,
        }
    }
}

#[derive(Debug)]
pub struct ViewEntry {
    pub filename: PathBuf,
    pub size: u64,
    pub status: EntryStatus,
    pub uri: quilt::uri::S3PackageUri,
    pub origin: Option<url::Url>,
    /// If Some, file is detected as junk — show eye-off button with this pattern
    pub junky_pattern: Option<String>,
    /// If Some, file is already ignored by .quiltignore — show eye-on button
    pub ignored_by: Option<String>,
}

#[derive(Template)]
#[template(source = "...", ext = "txt")]
pub struct TmplCheckbox {
    disabled: bool,
    checked: bool,
    // TODO: js selector
}

#[derive(Template)]
#[template(path = "./components/entry.html")]
pub struct TmplEntry<'a> {
    filename: PathBuf,
    status: EntryStatus,
    size: u64,
    checkbox: Option<TmplCheckbox>,
    actions: Vec<btn::TmplButton<'a>>,
    junky: bool,
    ignored: bool,
}

impl From<ViewEntry> for TmplEntry<'_> {
    fn from(view: ViewEntry) -> Self {
        let junky = view.junky_pattern.is_some();
        let ignored = view.ignored_by.is_some();

        let mut actions = Vec::new();
        if !matches!(view.status, EntryStatus::Remote)
            && !matches!(view.status, EntryStatus::Deleted)
            && !ignored
        {
            actions.push(
                btn::TmplButton::builder()
                    .set_js(btn::JsSelector::OpenInDefaultApplication)
                    .set_label(t!("buttons.open_entry_in_default_application"))
                    .set_size(btn::Size::Small)
                    .set_data("namespace", view.uri.namespace.to_string())
                    .set_data("path", view.filename.display().to_string())
                    .set_icon(Icon::OpenInNew),
            );
            actions.push(
                btn::TmplButton::builder()
                    .set_js(btn::JsSelector::RevealInFileBrowser)
                    .set_label(t!("buttons.reveal_entry_in_file_browser"))
                    .set_size(btn::Size::Small)
                    .set_data("namespace", view.uri.namespace.to_string())
                    .set_data("path", view.filename.display().to_string())
                    .set_icon(Icon::FolderOpen),
            );
        }
        if let Some(origin) = view.origin {
            if matches!(view.status, EntryStatus::Remote)
                || matches!(view.status, EntryStatus::Pristine)
            {
                actions.push(
                    btn::TmplButton::builder()
                        .set_js(btn::JsSelector::OpenInWebBrowser)
                        .set_label(t!("buttons.open_entry_in_catalog"))
                        .set_size(btn::Size::Small)
                        .set_data("url", origin.to_string())
                        .set_icon(Icon::OpenInBrowser),
                );
            }
        }

        // Eye-off button for junky files (suggest ignoring)
        if let Some(pattern) = &view.junky_pattern {
            actions.push(
                btn::TmplButton::builder()
                    .set_js(btn::JsSelector::IgnoreEntry)
                    .set_label("Ignore")
                    .set_size(btn::Size::Small)
                    .set_data("namespace", view.uri.namespace.to_string())
                    .set_data("path", view.filename.display().to_string())
                    .set_data("pattern", pattern.clone())
                    .set_icon(Icon::VisibilityOff),
            );
        }

        // Eye-on button for ignored files (show why / edit)
        if let Some(pattern) = &view.ignored_by {
            actions.push(
                btn::TmplButton::builder()
                    .set_js(btn::JsSelector::UnignoreEntry)
                    .set_label("Ignored")
                    .set_size(btn::Size::Small)
                    .set_data("namespace", view.uri.namespace.to_string())
                    .set_data("pattern", pattern.clone())
                    .set_icon(Icon::Visibility),
            );
        }

        TmplEntry {
            filename: view.filename,
            status: view.status,
            size: view.size,
            checkbox: None,
            actions,
            junky,
            ignored,
        }
    }
}

impl TmplEntry<'_> {
    fn get_class_name_modificators(&self) -> String {
        let mut classes = vec![match self.status {
            EntryStatus::Added => "added",
            EntryStatus::Deleted => "deleted",
            EntryStatus::Modified => "modified",
            EntryStatus::Pristine => "pristine",
            EntryStatus::Remote => "remote",
        }];
        if self.junky {
            classes.push("junky");
        }
        if self.ignored {
            classes.push("ignored");
        }
        classes.join(" ")
    }

    fn display_status(&self) -> &str {
        match self.status {
            EntryStatus::Added => "New",
            EntryStatus::Deleted => "Deleted",
            EntryStatus::Modified => "Modified",
            EntryStatus::Pristine => "Downloaded",
            EntryStatus::Remote => "Remote",
        }
    }

    fn display_size(&self) -> String {
        humansize::format_size(self.size, humansize::DECIMAL)
    }

    pub fn set_checkbox(mut self, checked: bool) -> Self {
        let is_remote = matches!(self.status, EntryStatus::Remote);
        self.checkbox = Some(TmplCheckbox {
            disabled: !is_remote,
            checked: checked || !is_remote,
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::quilt::manifest::ManifestRow;

    use crate::Result;

    fn create_entry() -> ViewEntry {
        ViewEntry {
            filename: PathBuf::from("TEST"),
            origin: Some(
                url::Url::parse("https://test.quilt.dev/b/b/packages/a/b/tree/latest/TEST")
                    .unwrap(),
            ),
            size: 0,
            status: EntryStatus::Modified,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://b#package=a/b").unwrap(),
            junky_pattern: None,
            ignored_by: None,
        }
    }

    #[test]
    fn test_entry_status_from_change() {
        let added_change = Change::Added(ManifestRow {
            logical_key: PathBuf::from("test.txt"),
            ..ManifestRow::default()
        });
        let modified_change = Change::Modified(ManifestRow {
            logical_key: PathBuf::from("test.txt"),
            ..ManifestRow::default()
        });
        let removed_change = Change::Removed(ManifestRow {
            logical_key: PathBuf::from("test.txt"),
            ..ManifestRow::default()
        });

        assert!(matches!(
            EntryStatus::from(&added_change),
            EntryStatus::Added
        ));
        assert!(matches!(
            EntryStatus::from(&modified_change),
            EntryStatus::Modified
        ));
        assert!(matches!(
            EntryStatus::from(&removed_change),
            EntryStatus::Deleted
        ));
    }

    #[test]
    fn test_entry_display_status() {
        let entry_added = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Added,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_deleted = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Deleted,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_modified = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Modified,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_pristine = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Pristine,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_remote = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Remote,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        assert_eq!(entry_added.display_status(), "New");
        assert_eq!(entry_deleted.display_status(), "Deleted");
        assert_eq!(entry_modified.display_status(), "Modified");
        assert_eq!(entry_pristine.display_status(), "Downloaded");
        assert_eq!(entry_remote.display_status(), "Remote");
    }

    #[test]
    fn test_entry_class_name_modificators() {
        let entry_added = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Added,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_deleted = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Deleted,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_modified = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Modified,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_pristine = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Pristine,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_remote = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Remote,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        assert_eq!(entry_added.get_class_name_modificators(), "added");
        assert_eq!(entry_deleted.get_class_name_modificators(), "deleted");
        assert_eq!(entry_modified.get_class_name_modificators(), "modified");
        assert_eq!(entry_pristine.get_class_name_modificators(), "pristine");
        assert_eq!(entry_remote.get_class_name_modificators(), "remote");
    }

    #[test]
    fn test_set_checkbox() {
        // Test with remote status
        let entry_remote = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Remote,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_with_checkbox = entry_remote.set_checkbox(true);
        assert!(entry_with_checkbox.checkbox.is_some());
        if let Some(checkbox) = entry_with_checkbox.checkbox {
            assert!(!checkbox.disabled);
            assert!(checkbox.checked);
        }

        // Test with non-remote status
        let entry_modified = TmplEntry {
            filename: PathBuf::from("test.txt"),
            status: EntryStatus::Modified,
            size: 0,
            checkbox: None,
            actions: vec![],
            junky: false,
            ignored: false,
        };

        let entry_with_checkbox = entry_modified.set_checkbox(false);
        assert!(entry_with_checkbox.checkbox.is_some());
        if let Some(checkbox) = entry_with_checkbox.checkbox {
            assert!(checkbox.disabled);
            assert!(checkbox.checked); // Should be checked regardless of input for non-remote
        }
    }

    #[test]
    fn test_display_size() {
        let sizes = [
            (0, "0 B"),
            (12, "12 B"),
            (1234, "1.23 kB"),
            (12345678, "12.35 MB"),
            (1234567890123456, "1.23 PB"),
            (12345678901234567890, "12.35 EB"),
        ];

        for (size_bytes, expected_str) in sizes.iter() {
            let entry = TmplEntry {
                filename: PathBuf::from("test.txt"),
                status: EntryStatus::Pristine,
                size: *size_bytes,
                checkbox: None,
                actions: vec![],
                junky: false,
                ignored: false,
            };

            assert_eq!(entry.display_size(), *expected_str);
        }
    }

    #[test]
    fn test_from_view_entry_to_tmpl_entry() {
        // Test with modified status (should have no actions)
        let view_entry_modified = create_entry();
        let tmpl_entry = TmplEntry::from(view_entry_modified);

        assert_eq!(tmpl_entry.filename.to_str().unwrap(), "TEST");
        assert_eq!(tmpl_entry.size, 0);
        assert!(matches!(tmpl_entry.status, EntryStatus::Modified));
        assert_eq!(tmpl_entry.actions.len(), 2); // Open and Reveal

        // Test with remote status (should have open and reveal actions)
        let mut view_entry_remote = create_entry();
        view_entry_remote.status = EntryStatus::Remote;
        let tmpl_entry = TmplEntry::from(view_entry_remote);

        assert_eq!(tmpl_entry.actions.len(), 1); // Only Open in Catalog

        // Test with pristine status (should have open in catalog action)
        let mut view_entry_pristine = create_entry();
        view_entry_pristine.status = EntryStatus::Pristine;
        let tmpl_entry = TmplEntry::from(view_entry_pristine);

        assert_eq!(tmpl_entry.actions.len(), 3); // Open, Reveal and Open in Catalog

        // Test with pristine status (should have open in catalog action)
        let mut view_entry_pristine = create_entry();
        view_entry_pristine.status = EntryStatus::Deleted;
        let tmpl_entry = TmplEntry::from(view_entry_pristine);

        assert_eq!(tmpl_entry.actions.len(), 0); // No buttons
    }

    #[test]
    fn test_entry_html_rendering() -> Result<()> {
        // Create a remote entry which should have all action buttons
        let mut view_entry = create_entry();
        view_entry.status = EntryStatus::Pristine;

        let tmpl_entry = TmplEntry::from(view_entry);
        let html = tmpl_entry.to_string();

        // Check for basic structure and content
        assert!(html.contains("TEST")); // Filename
        assert!(html.contains("Downloaded")); // Status

        // Check for action buttons
        assert!(html.contains(r#"js-open-in-default-application"#));
        assert!(html.contains(r#"js-reveal-in-file-browser"#));
        assert!(html.contains(r#"js-open-in-web-browser"#));

        // Check for data attributes
        assert!(html.contains(r#"data-namespace="a/b""#));
        assert!(html.contains(r#"data-path="TEST""#));

        Ok(())
    }

    #[test]
    fn test_checkbox_rendering() -> Result<()> {
        // Create a remote entry with checkbox
        let mut view_entry = create_entry();
        view_entry.status = EntryStatus::Remote;

        let tmpl_entry = TmplEntry::from(view_entry).set_checkbox(true);
        let html = tmpl_entry.to_string();

        // Check for checkbox
        assert!(html.contains("checkbox"));
        assert!(html.contains("checked"));

        // Create a non-remote entry with checkbox
        let view_entry = create_entry(); // Modified by default
        let tmpl_entry = TmplEntry::from(view_entry).set_checkbox(false);
        let html = tmpl_entry.to_string();

        // Check for disabled checkbox
        assert!(html.contains("checkbox"));
        assert!(html.contains("disabled"));
        assert!(html.contains("checked")); // Should be checked regardless of input

        Ok(())
    }
}

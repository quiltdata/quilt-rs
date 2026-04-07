use std::borrow::Cow;
use std::collections::BTreeMap;

use askama::Template;

use crate::routes::Paths;
use crate::ui::Icon;

#[derive(Default)]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
}

#[derive(Default)]
pub enum ButtonType {
    #[default]
    Button,
    Submit,
}

#[derive(Template)]
pub enum JsSelector {
    #[template(source = "js-erase-auth", ext = "txt")]
    EraseAuth,
    #[template(source = "js-debug-dot-quilt", ext = "txt")]
    DotQuilt,
    #[template(source = "js-debug-logs", ext = "txt")]
    DebugLogs,
    #[template(source = "js-packages-certify-latest", ext = "txt")]
    CertifyLatest,
    #[template(source = "js-packages-reset-local", ext = "txt")]
    ResetLocal,
    #[template(source = "js-login", ext = "txt")]
    Login,
    #[template(source = "js-login-oauth", ext = "txt")]
    LoginOAuth,
    #[template(source = "js-entries-install", ext = "txt")]
    EntriesInstall,
    #[template(source = "js-refresh", ext = "txt")]
    Refresh,
    #[template(source = "js-open-in-default-application", ext = "txt")]
    OpenInDefaultApplication,
    #[template(source = "js-open-in-file-browser", ext = "txt")]
    OpenInFileBrowser,
    #[template(source = "js-open-in-web-browser", ext = "txt")]
    OpenInWebBrowser,
    #[template(source = "js-release-notes", ext = "txt")]
    ReleaseNotes,
    #[template(source = "js-packages-push", ext = "txt")]
    PackagesPush,
    #[template(source = "js-packages-pull", ext = "txt")]
    PackagesPull,
    #[template(source = "js-packages-uninstall", ext = "txt")]
    PackagesUninstall,
    #[template(source = "js-packages-commit", ext = "txt")]
    PackagesCommit,
    #[template(source = "js-setup", ext = "txt")]
    Setup,
    #[template(source = "js-open-directory-picker", ext = "txt")]
    DirectoryPicker,
    #[template(source = "js-reveal-in-file-browser", ext = "txt")]
    RevealInFileBrowser,
    #[template(source = "js-set-origin", ext = "txt")]
    SetOrigin,
    #[template(source = "js-set-remote", ext = "txt")]
    SetRemote,
    #[template(source = "js-create-package", ext = "txt")]
    CreatePackage,
    #[template(source = "js-crash-report", ext = "txt")]
    CrashReport,
    #[template(source = "js-collect-logs", ext = "txt")]
    CollectLogs,
    #[template(source = "js-diagnostic-logs", ext = "txt")]
    DiagnosticLogs,
    #[template(source = "js-open-home-dir", ext = "txt")]
    OpenHomeDir,
    #[template(source = "js-open-data-dir", ext = "txt")]
    OpenDataDir,
    #[template(source = "js-ignore-entry", ext = "txt")]
    IgnoreEntry,
    #[template(source = "js-unignore-entry", ext = "txt")]
    UnignoreEntry,
}

#[derive(Template)]
pub enum Color {
    #[template(source = "primary", ext = "txt")]
    Primary,
    #[template(source = "warning", ext = "txt")]
    Warning,
}

#[derive(Template)]
pub enum Modificator {
    #[template(source = "link", ext = "txt")]
    Link,
}

#[derive(Template)]
pub enum Size {
    #[template(source = "small", ext = "txt")]
    Small,
    #[template(source = "large", ext = "txt")]
    Large,
}

#[derive(Template, Default)]
#[template(path = "./components/button.html")]
pub struct TmplButton<'a> {
    color: Option<Color>,
    data: Option<BTreeMap<Cow<'a, str>, Cow<'a, str>>>,
    direction: Direction,
    button_type: ButtonType,
    disabled: bool,
    href: Option<Paths>,
    icon: Option<Icon>,
    js: Option<JsSelector>,
    label: Cow<'a, str>,
    modificator: Option<Modificator>,
    size: Option<Size>,
    title: Option<Cow<'a, str>>,
}

impl<'a> TmplButton<'a> {
    pub fn builder() -> TmplButton<'a> {
        TmplButton {
            direction: Direction::default(),
            ..TmplButton::default()
        }
    }

    pub fn set_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn set_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn set_js(mut self, js: JsSelector) -> Self {
        self.js = Some(js);
        self
    }

    pub fn set_label<T: Into<Cow<'a, str>>>(mut self, label: T) -> Self {
        self.label = label.into();
        self
    }

    pub fn set_modificator(mut self, modificator: Modificator) -> Self {
        self.modificator = Some(modificator);
        self
    }

    pub fn set_size(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    pub fn set_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn set_data<K: Into<Cow<'a, str>>, V: Into<Cow<'a, str>>>(
        mut self,
        key: K,
        value: V,
    ) -> Self {
        self.data = Some(match self.data {
            Some(mut data) => {
                data.insert(key.into(), value.into());
                data
            }
            None => BTreeMap::from([(key.into(), value.into())]),
        });
        self
    }

    pub fn set_href(mut self, href: Paths) -> Self {
        self.href = Some(href);
        self
    }

    pub fn set_disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn set_type(mut self, button_type: ButtonType) -> Self {
        self.button_type = button_type;
        self
    }

    pub fn set_title<T: Into<Cow<'a, str>>>(mut self, title: T) -> Self {
        self.title = Some(title.into());
        self
    }

    fn get_class_name_modificators(&self) -> String {
        let mut classes = Vec::new();
        if let Some(color) = &self.color {
            classes.push(color.to_string());
        }
        if let Some(modificator) = &self.modificator {
            classes.push(modificator.to_string());
        }
        if let Some(js) = &self.js {
            classes.push(js.to_string());
        }
        if let Some(size) = &self.size {
            classes.push(size.to_string());
        }
        classes.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::strip_whitespace;

    #[test]
    fn test_button_builder_default() {
        let button = TmplButton::builder().set_label("Default button");

        // Default button should have empty label and no options set
        let html = button
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(html.contains(
            r#"<button class="qui-button " type="button"><span>Default button</span></button>"#
        ));
        assert!(!html.contains("data-"));
        assert!(!html.contains("disabled"));
        assert!(!html.contains(r#"<img class="qui-icon""#));
    }

    #[test]
    fn test_button_with_label() {
        let button = TmplButton::builder().set_label("Test Button");

        let html = button.to_string();
        assert!(html.contains("<span>Test Button</span>"));
    }

    #[test]
    fn test_button_with_icon() {
        let button = TmplButton::builder()
            .set_label("Button with Icon")
            .set_icon(Icon::Done);

        let html = button.to_string();
        assert!(html.contains(r#"<img class="qui-icon""#));
        assert!(html.contains("<span>Button with Icon</span>"));
    }

    #[test]
    fn test_button_with_js_selector() {
        let button = TmplButton::builder()
            .set_label("Refresh")
            .set_js(JsSelector::Refresh);

        let html = button.to_string();
        assert!(html.contains(r#"class="qui-button js-refresh""#));
    }

    #[test]
    fn test_button_with_color() {
        let button = TmplButton::builder()
            .set_label("Primary Button")
            .set_color(Color::Primary);

        let html = button.to_string();
        assert!(html.contains(r#"class="qui-button primary""#));
    }

    #[test]
    fn test_button_with_modificator() {
        let button = TmplButton::builder()
            .set_label("Link Button")
            .set_modificator(Modificator::Link);

        let html = button.to_string();
        assert!(html.contains(r#"class="qui-button link""#));
    }

    #[test]
    fn test_button_with_size() {
        let button_small = TmplButton::builder()
            .set_label("Small Button")
            .set_size(Size::Small);

        let button_large = TmplButton::builder()
            .set_label("Large Button")
            .set_size(Size::Large);

        let html_small = button_small.to_string();
        let html_large = button_large.to_string();

        assert!(html_small.contains(r#"class="qui-button small""#));
        assert!(html_large.contains(r#"class="qui-button large""#));
    }

    #[test]
    fn test_button_with_data_attribute() {
        let button = TmplButton::builder()
            .set_label("Data Button")
            .set_data("test-key", "test-value");

        let html = button.to_string();
        assert!(html.contains(r#"data-test-key="test-value""#));
    }

    #[test]
    fn test_button_with_multiple_data_attributes() {
        let button = TmplButton::builder()
            .set_label("Multi Data Button")
            .set_data("key1", "value1")
            .set_data("key2", "value2");

        let html = button.to_string();
        assert!(html.contains(r#"data-key1="value1""#));
        assert!(html.contains(r#"data-key2="value2""#));
    }

    #[test]
    fn test_disabled_button() {
        let button = TmplButton::builder()
            .set_label("Disabled Button")
            .set_disabled();

        let html = strip_whitespace(button.to_string());
        assert!(html.contains(r#"disabled type="button">"#));
    }

    #[test]
    fn test_button_with_href() {
        let button = TmplButton::builder()
            .set_label("Link Button")
            .set_href(Paths::InstalledPackagesList);

        let html = button.to_string();
        assert!(html.contains(r#"<a href="installed-packages-list.html">"#));
        assert!(html.contains("</a>"));
    }

    #[test]
    fn test_get_class_name_modificators() {
        let button = TmplButton::builder()
            .set_color(Color::Primary)
            .set_modificator(Modificator::Link)
            .set_js(JsSelector::Refresh)
            .set_size(Size::Large);

        let class_names = button.get_class_name_modificators();

        // Check that all modifiers are included in the class string
        assert!(class_names.contains("primary"));
        assert!(class_names.contains("link"));
        assert!(class_names.contains("js-refresh"));
        assert!(class_names.contains("large"));
    }

    #[test]
    fn test_full_button_rendering() {
        let button = TmplButton::builder()
            .set_label("Complete Button")
            .set_icon(Icon::Done)
            .set_color(Color::Primary)
            .set_js(JsSelector::PackagesCommit)
            .set_size(Size::Large)
            .set_data("form", "#form")
            .set_href(Paths::InstalledPackagesList);

        let snapshot = r##"<style> @import url("/assets/css/components/button.css"); </style><a href="installed-packages-list.html"><button class="qui-button primary js-packages-commit large" data-form="#form" type="button"><img class="qui-icon" src="/assets/img/icons/done.svg" /><span>Complete Button</span></button></a>"##;

        let html = button.to_string();
        assert_eq!(strip_whitespace(html), snapshot)
    }
}

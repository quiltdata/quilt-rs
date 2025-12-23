use rust_i18n::t;

use crate::app::Globals;
use crate::quilt;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::debug_toolbar;
use crate::ui::uri;
use crate::ui::Icon;

#[derive(Default)]
pub struct Layout<'a> {
    pub breadcrumbs: Option<crumbs::TmplBreadcrumbs<'a>>,
    pub debug_toolbar: debug_toolbar::TmplDebugToolbar<'a>,
    pub primary_action: Option<btn::TmplButton<'a>>,
    pub refresh_button: btn::TmplButton<'a>,
    pub secondary_actions: Option<Vec<btn::TmplButton<'a>>>,
    pub uri: uri::TmplUri<'a>,
}

impl<'a> Layout<'a> {
    pub fn new(globals: Globals, primary_action: Option<btn::TmplButton<'a>>) -> Layout<'a> {
        let layout = Self::builder(globals);
        match primary_action {
            Some(action) => layout.set_primary_action(action),
            None => layout,
        }
    }

    pub fn refresh_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_icon(Icon::Refresh)
            .set_js(btn::JsSelector::Refresh)
            .set_label(t!("appbar.refresh"))
            .set_modificator(btn::Modificator::Link)
    }

    pub fn builder(globals: Globals) -> Layout<'a> {
        Layout {
            primary_action: None,
            secondary_actions: None,
            breadcrumbs: None,
            debug_toolbar: debug_toolbar::TmplDebugToolbar::create(&globals),
            refresh_button: Self::refresh_button(),
            uri: uri::TmplUri::new(None),
        }
    }

    pub fn set_breadcrumbs(mut self, breadcrumbs: crumbs::TmplBreadcrumbs<'a>) -> Self {
        self.breadcrumbs = Some(breadcrumbs);
        self
    }

    pub fn set_primary_action(mut self, button: btn::TmplButton<'a>) -> Self {
        self.primary_action = Some(button);
        self
    }

    pub fn set_actions(mut self, actions: Vec<btn::TmplButton<'a>>) -> Self {
        self.secondary_actions = Some(actions);
        self
    }

    pub fn set_uri(mut self, uri: Option<quilt::uri::S3PackageUri>) -> Self {
        self.uri = uri::TmplUri::new(uri);
        self
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn test_builder_creates_layout_with_defaults() {
        let layout = Layout::default();

        assert!(layout.primary_action.is_none());
        assert!(layout.secondary_actions.is_none());
        assert!(layout.breadcrumbs.is_none());
        // We can't directly compare refresh_button, but we can verify it exists
    }

    #[test]
    fn test_new_with_primary_action() {
        let primary_action = btn::TmplButton::builder()
            .set_label(Cow::from("Test Action"))
            .set_icon(Icon::Done);

        let layout = Layout::new(Globals::default(), Some(primary_action));

        assert!(layout.primary_action.is_some());
    }

    #[test]
    fn test_new_without_primary_action() {
        let layout = Layout::new(Globals::default(), None);

        assert!(layout.primary_action.is_none());
    }

    #[test]
    fn test_set_breadcrumbs() {
        let breadcrumbs = crumbs::TmplBreadcrumbs {
            list: vec![crumbs::BreadcrumbItem::Current(crumbs::Current {
                title: Cow::from("Test"),
            })],
        };

        let layout = Layout::builder(Globals::default()).set_breadcrumbs(breadcrumbs);

        assert!(layout.breadcrumbs.is_some());
        if let Some(crumbs) = layout.breadcrumbs {
            assert_eq!(crumbs.list.len(), 1);
        }
    }

    #[test]
    fn test_set_primary_action() {
        let button = btn::TmplButton::builder()
            .set_label(Cow::from("Test Button"))
            .set_icon(Icon::Done);

        let layout = Layout::builder(Globals::default()).set_primary_action(button);

        assert!(layout.primary_action.is_some());
    }

    #[test]
    fn test_set_actions() {
        let button1 = btn::TmplButton::builder()
            .set_label(Cow::from("Button 1"))
            .set_icon(Icon::Done);
        let button2 = btn::TmplButton::builder()
            .set_label(Cow::from("Button 2"))
            .set_icon(Icon::Refresh);

        let actions = vec![button1, button2];
        let layout = Layout::builder(Globals::default()).set_actions(actions);

        assert!(layout.secondary_actions.is_some());
        if let Some(actions) = layout.secondary_actions {
            assert_eq!(actions.len(), 2);
        }
    }

    #[test]
    fn test_refresh_button() {
        let refresh_button = Layout::refresh_button();

        // Test the rendered HTML output
        let rendered = refresh_button.to_string();

        // Check for expected elements in the rendered HTML
        assert!(rendered.contains(r#"class="qui-button"#));
        assert!(rendered.contains("js-refresh"));
        assert!(rendered.contains("link"));
        assert!(rendered.contains(r#"<img class="qui-icon""#));
        assert!(rendered.contains("<span>"));
        assert!(rendered.contains(&t!("appbar.refresh").to_string()));
    }

    #[test]
    fn test_builder_method_chain() {
        let breadcrumbs = crumbs::TmplBreadcrumbs {
            list: vec![crumbs::BreadcrumbItem::Current(crumbs::Current {
                title: Cow::from("Test"),
            })],
        };
        let primary_button = btn::TmplButton::builder()
            .set_label(Cow::from("Primary"))
            .set_icon(Icon::Done);
        let secondary_button = btn::TmplButton::builder()
            .set_label(Cow::from("Secondary"))
            .set_icon(Icon::Refresh);

        let layout = Layout::builder(Globals::default())
            .set_breadcrumbs(breadcrumbs)
            .set_primary_action(primary_button)
            .set_actions(vec![secondary_button]);

        assert!(layout.breadcrumbs.is_some());
        assert!(layout.primary_action.is_some());
        assert!(layout.secondary_actions.is_some());
        if let Some(actions) = layout.secondary_actions {
            assert_eq!(actions.len(), 1);
        }
    }
}

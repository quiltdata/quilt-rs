use std::path::Path;
use std::path::PathBuf;

use askama::Template;
use rust_i18n::t;

use crate::app::AppAssets;
use crate::app::Globals;
use crate::error::Error;
use crate::ui::btn;
use crate::ui::layout::Layout;
use crate::ui::Icon;

#[derive(Debug)]
pub struct ViewSetup {
    globals: Globals,
    home: PathBuf,
}

#[derive(Template)]
#[template(path = "./pages/setup.html")]
pub struct TmplSetup<'a> {
    browse: btn::TmplButton<'a>,
    home: PathBuf,
    layout: Layout<'a>,
}

impl<'a> TmplSetup<'a> {
    pub fn primary_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::Setup)
            .set_icon(Icon::Done)
            .set_data("form", "#form")
            .set_label(t!("setup.submit"))
            .set_color(btn::Color::Primary)
            .set_size(btn::Size::Large)
    }

    pub fn directory_picker() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::DirectoryPicker)
            .set_data("target", "#directory")
            .set_label(t!("setup.browse"))
    }
}

impl From<ViewSetup> for TmplSetup<'_> {
    fn from(view: ViewSetup) -> Self {
        TmplSetup {
            browse: Self::directory_picker(),
            home: view.home,
            layout: Layout::new(view.globals, Some(Self::primary_button())),
        }
    }
}

impl ViewSetup {
    pub async fn create(app: &impl AppAssets, home: &Path) -> Result<ViewSetup, Error> {
        Ok(ViewSetup {
            home: home.to_path_buf(),
            globals: app.globals(),
        })
    }

    pub fn render(self) -> Result<String, Error> {
        Ok(TmplSetup::from(self)
            .render()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn test_setup_page_rendering() -> Result<()> {
        // Create a test home directory
        let home_dir = PathBuf::from("/home/user/QuiltSync");

        // Create the view
        let view = ViewSetup {
            globals: Globals::default(),
            home: home_dir.clone(),
        };

        // Render the view to HTML
        let html = view.render()?;

        // Check for directory input field
        assert!(html.contains(r#"<input class="input" id="directory" name="directory" required readonly value="/home/user/QuiltSync" />"#));

        assert!(html.contains(r##"<button class="qui-button js-open-directory-picker" data-target="#directory" type="button"><span>Browse</span></button>"##));

        assert!(html.contains(r##"<button class="qui-button primary js-setup large" data-form="#form" type="button"><img class="qui-icon" src="/assets/img/icons/done.svg" /><span>Save</span></button>"##));

        Ok(())
    }
}

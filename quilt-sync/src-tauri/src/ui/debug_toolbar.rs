use askama::Template;
use rust_i18n::t;

use crate::app::Globals;
use crate::ui::btn;
use crate::ui::Icon;

#[derive(Template, Default)]
#[template(path = "./components/debug-toolbar.html")]
pub struct TmplDebugToolbar<'a> {
    button: btn::TmplButton<'a>,
    dot_quilt_button: btn::TmplButton<'a>,
    auth_button: btn::TmplButton<'a>,
    logs_dir_button: btn::TmplButton<'a>,
    release_notes: btn::TmplButton<'a>,
    reload_button: btn::TmplButton<'a>,
    version: String,
}

impl<'a> TmplDebugToolbar<'a> {
    pub fn create(globals: &Globals) -> Self {
        let version = globals.version.to_string();
        TmplDebugToolbar {
            button: Self::button(globals),
            dot_quilt_button: Self::dot_quilt_button(),
            auth_button: Self::auth_button(),
            logs_dir_button: Self::logs_dir_button(globals),
            release_notes: Self::release_notes_button(globals),
            reload_button: Self::reload_button(),
            version,
        }
    }

    fn release_notes_button(globals: &Globals) -> btn::TmplButton<'static> {
        let release_url = format!(
            "https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v{}",
            globals.version
        );
        btn::TmplButton::builder()
            .set_label(t!("debug_toolbar.release_notes"))
            .set_modificator(btn::Modificator::Link)
            .set_js(btn::JsSelector::OpenInWebBrowser)
            .set_data("url", release_url.clone())
            .set_title(release_url)
    }

    fn logs_dir_button(globals: &Globals) -> btn::TmplButton<'static> {
        let logs_dir_path = globals.logs_dir.display().to_string();
        let button = btn::TmplButton::builder()
            .set_label(t!("debug_toolbar.show_logs"))
            .set_modificator(btn::Modificator::Link)
            .set_js(btn::JsSelector::DebugLogs)
            .set_title(logs_dir_path);

        if globals.logs_dir_is_temporary {
            return button.set_icon(Icon::Warning);
        }

        button
    }

    fn button(globals: &Globals) -> btn::TmplButton<'static> {
        let button = btn::TmplButton::builder().set_size(btn::Size::Small);

        if globals.logs_dir_is_temporary {
            button.set_icon(Icon::Warning)
        } else {
            button.set_icon(Icon::Gear)
        }
    }

    fn dot_quilt_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::DotQuilt)
            .set_modificator(btn::Modificator::Link)
            .set_label(t!("debug_toolbar.dot_quilt_button"))
    }

    fn reload_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::Refresh)
            .set_modificator(btn::Modificator::Link)
            .set_label(t!("debug_toolbar.reload_page"))
    }

    fn auth_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::EraseAuth)
            .set_modificator(btn::Modificator::Link)
            .set_label(t!("debug_toolbar.reset_auth"))
    }
}

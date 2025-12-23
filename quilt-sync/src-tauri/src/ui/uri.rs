use askama::Template;

use crate::quilt;
use crate::ui::btn;
use crate::ui::Icon;

#[derive(Template, Default)]
#[template(path = "./components/uri.html")]
pub struct TmplUri<'a> {
    pub submit_button: btn::TmplButton<'a>,
    pub uri: Option<quilt::uri::S3PackageUri>,
}

impl<'a> TmplUri<'a> {
    pub fn new(uri: Option<quilt::uri::S3PackageUri>) -> Self {
        Self {
            submit_button: Self::create_submit_button(),
            uri,
        }
    }

    fn create_submit_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_icon(Icon::ArrowForward)
            .set_type(btn::ButtonType::Submit)
            .set_data("form", "#uri")
            .set_color(btn::Color::Primary)
            .set_modificator(btn::Modificator::Link)
    }
}

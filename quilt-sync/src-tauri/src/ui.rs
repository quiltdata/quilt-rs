pub mod breadcrumbs;
pub mod button;
pub mod changelog;
pub mod entry;
pub mod icon;
pub mod layout;
pub mod notify;
pub mod uri;

pub use breadcrumbs as crumbs;
pub use button as btn;
pub use icon::Icon;

pub fn strip_whitespace(html: impl AsRef<str>) -> String {
    html.as_ref()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

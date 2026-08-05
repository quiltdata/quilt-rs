//! The UI kit — hand-written components for the v2 main page.
//!
//! Named `kit` because v1 still owns `components`; when v1 retires, that module
//! goes with it and nothing here needs renaming.
//!
//! **Components here are presentation only:** props in, view out, callbacks for
//! anything with an effect. No `commands`, no `tauri`, no fetching. That is
//! enforced rather than encouraged — `gallery.rs` mounts this module in a plain
//! browser with no Tauri runtime, so a component that calls a command cannot
//! render, and therefore cannot be reviewed.
//!
//! Each component is a `<name>.rs` with its stylesheet beside it as
//! `<name>.module.scss` (see `Cargo.toml` for how those are bundled).

pub mod button;
pub mod card;
pub mod countdown;
pub mod host_row;
pub mod search_input;
pub mod select;
pub mod toggle_row;
pub mod view_toggle;

pub use button::Button;
pub use button::ButtonSize;
pub use button::ButtonVariant;
pub use card::Card;
pub use countdown::Countdown;
pub use host_row::HostRow;
pub use search_input::SearchInput;
pub use select::Select;
pub use toggle_row::ToggleRow;
pub use view_toggle::ViewToggle;

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
pub mod cause_row;
pub mod countdown;
pub mod empty_state;
pub mod file_row;
pub mod group_header;
pub mod host_row;
pub mod icon_button;
pub mod list_toolbar;
pub mod package_row;
pub mod queue_row;
pub mod section_label;
pub mod relative_time;
pub mod search_input;
pub mod select;
pub mod state_label;
pub mod toggle_row;
pub mod view_toggle;
pub mod zero_line;

pub use button::Button;
pub use button::ButtonSize;
pub use button::ButtonVariant;
pub use card::Card;
pub use cause_row::CauseRow;
pub use countdown::Countdown;
pub use empty_state::EmptyState;
pub use file_row::FileRow;
pub use group_header::GroupHeader;
pub use host_row::HostRow;
pub use icon_button::IconButton;
pub use icon_button::IconButtonVariant;
pub use list_toolbar::ListToolbar;
pub use package_row::PackageRow;
pub use queue_row::QueueRow;
pub use section_label::SectionLabel;
pub use relative_time::RelativeTime;
pub use search_input::SearchInput;
pub use select::Select;
pub use state_label::StateLabel;
pub use state_label::StateTone;
pub use toggle_row::ToggleRow;
pub use view_toggle::ViewToggle;
pub use zero_line::ZeroLine;

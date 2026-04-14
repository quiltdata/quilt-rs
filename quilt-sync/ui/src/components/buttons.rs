mod icon_button;
pub mod commit;
pub mod create_local_package;
pub mod ignore;
pub mod login;
pub mod merge;
pub mod open;
pub mod open_in_catalog;
pub mod open_in_file_browser;
pub mod pull;
pub mod push;
pub mod remove;
pub mod reveal;
pub mod set_origin;
pub mod set_remote;
pub mod unignore;

pub use commit::Commit;
pub use create_local_package::CreateLocalPackage;
pub use ignore::Ignore;
pub use login::Login;
pub use merge::Merge;
pub use open::Open;
pub use open_in_catalog::OpenInCatalog;
pub use open_in_file_browser::OpenInFileBrowser;
pub use pull::Pull;
pub use push::Push;
pub use remove::Remove;
pub use reveal::Reveal;
pub use set_origin::SetOrigin;
pub use set_remote::SetRemote;
pub use unignore::Unignore;

use icon_button::IconButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Commit,
    CreateLocalPackage,
    Ignore,
    Login,
    Merge,
    Open,
    OpenInCatalog,
    OpenInFileBrowser,
    Pull,
    Push,
    Remove,
    Reveal,
    SetOrigin,
    SetRemote,
    Unignore,
}

impl ButtonKind {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Commit => "/assets/img/icons/commit.svg",
            Self::CreateLocalPackage => "/assets/img/icons/add.svg",
            Self::Ignore => "/assets/img/icons/visibility_off.svg",
            Self::Login => "/assets/img/icons/warning.svg",
            Self::Merge => "/assets/img/icons/merge.svg",
            Self::Open => "/assets/img/icons/open_in_new.svg",
            Self::OpenInCatalog => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInFileBrowser => "/assets/img/icons/folder_open.svg",
            Self::Pull => "/assets/img/icons/cloud_download.svg",
            Self::Push => "/assets/img/icons/cloud_upload.svg",
            Self::Remove => "/assets/img/icons/block.svg",
            Self::Reveal => "/assets/img/icons/folder_open.svg",
            Self::SetOrigin => "/assets/img/icons/warning.svg",
            Self::SetRemote => "/assets/img/icons/cloud_upload.svg",
            Self::Unignore => "/assets/img/icons/visibility.svg",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Commit => "Commit",
            Self::CreateLocalPackage => "Create local package",
            Self::Ignore => "Ignore",
            Self::Login => "Login",
            Self::Merge => "Merge",
            Self::Open => "Open",
            Self::OpenInCatalog => "Open in Catalog",
            Self::OpenInFileBrowser => "Open",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Remove => "Remove",
            Self::Reveal => "Reveal",
            Self::SetOrigin => "Set origin",
            Self::SetRemote => "Set remote",
            Self::Unignore => "Ignored",
        }
    }
}

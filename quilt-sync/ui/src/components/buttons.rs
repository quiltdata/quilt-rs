mod icon_button;
mod icon_link;
pub mod commit;
pub mod create_local_package;
pub mod login;
pub mod merge;
pub mod open_in_catalog;
pub mod open_in_file_browser;
pub mod pull;
pub mod push;
pub mod remove;
pub mod set_origin;
pub mod set_remote;

pub use commit::Commit;
pub use create_local_package::CreateLocalPackage;
pub use login::Login;
pub use merge::Merge;
pub use open_in_catalog::OpenInCatalog;
pub use open_in_file_browser::OpenInFileBrowser;
pub use pull::Pull;
pub use push::Push;
pub use remove::Remove;
pub use set_origin::SetOrigin;
pub use set_remote::SetRemote;

use icon_button::IconButton;
use icon_link::IconLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Commit,
    CreateLocalPackage,
    Login,
    Merge,
    OpenInCatalog,
    OpenInFileBrowser,
    Pull,
    Push,
    Remove,
    SetOrigin,
    SetRemote,
}

impl ButtonKind {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Commit => "/assets/img/icons/commit.svg",
            Self::CreateLocalPackage => "/assets/img/icons/add.svg",
            Self::Login => "/assets/img/icons/warning.svg",
            Self::Merge => "/assets/img/icons/merge.svg",
            Self::OpenInCatalog => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInFileBrowser => "/assets/img/icons/folder_open.svg",
            Self::Pull => "/assets/img/icons/cloud_download.svg",
            Self::Push => "/assets/img/icons/cloud_upload.svg",
            Self::Remove => "/assets/img/icons/block.svg",
            Self::SetOrigin => "/assets/img/icons/warning.svg",
            Self::SetRemote => "/assets/img/icons/cloud_upload.svg",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Commit => "Commit",
            Self::CreateLocalPackage => "Create local package",
            Self::Login => "Login",
            Self::Merge => "Merge",
            Self::OpenInCatalog => "Open in Catalog",
            Self::OpenInFileBrowser => "Open",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Remove => "Remove",
            Self::SetOrigin => "Set origin",
            Self::SetRemote => "Set remote",
        }
    }
}

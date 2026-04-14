mod icon_button;
mod icon_link;
pub mod commit;
pub mod merge;
pub mod open_in_catalog;
pub mod open_in_file_browser;
pub mod pull;
pub mod push;

pub use commit::Commit;
pub use merge::Merge;
pub use open_in_catalog::OpenInCatalog;
pub use open_in_file_browser::OpenInFileBrowser;
pub use pull::Pull;
pub use push::Push;

use icon_button::IconButton;
use icon_link::IconLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Commit,
    Merge,
    Push,
    Pull,
    OpenInCatalog,
    OpenInFileBrowser,
}

impl ButtonKind {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Commit => "/assets/img/icons/commit.svg",
            Self::Merge => "/assets/img/icons/merge.svg",
            Self::Push => "/assets/img/icons/cloud_upload.svg",
            Self::Pull => "/assets/img/icons/cloud_download.svg",
            Self::OpenInCatalog => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInFileBrowser => "/assets/img/icons/folder_open.svg",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Commit => "Commit",
            Self::Merge => "Merge",
            Self::Push => "Push",
            Self::Pull => "Pull",
            Self::OpenInCatalog => "Open in Catalog",
            Self::OpenInFileBrowser => "Open",
        }
    }
}

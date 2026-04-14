mod button_cta;
mod icon_button;
pub mod browse;
pub mod change_origin;
pub mod commit;
pub mod commit_revision;
pub mod create_new_revision;
pub mod create_local_package;
pub mod ignore;
pub mod log_in_with_browser;
pub mod login;
pub mod merge;
pub mod open;
pub mod open_browser;
pub mod open_in_catalog;
pub mod open_in_file_browser;
pub mod pull;
pub mod push;
pub mod remove;
pub mod reveal;
pub mod save;
pub mod set_origin;
pub mod set_remote;
pub mod submit_login;
pub mod unignore;

pub use browse::Browse;
pub use change_origin::ChangeOrigin;
pub use commit::Commit;
pub use commit_revision::CommitRevision;
pub use create_local_package::CreateLocalPackage;
pub use create_new_revision::CreateNewRevision;
pub use ignore::Ignore;
pub use log_in_with_browser::LogInWithBrowser;
pub use login::Login;
pub use merge::Merge;
pub use open::Open;
pub use open_browser::OpenBrowser;
pub use open_in_catalog::OpenInCatalog;
pub use open_in_file_browser::OpenInFileBrowser;
pub use pull::Pull;
pub use push::Push;
pub use remove::Remove;
pub use reveal::Reveal;
pub use save::Save;
pub use set_origin::SetOrigin;
pub use set_remote::SetRemote;
pub use submit_login::SubmitLogin;
pub use unignore::Unignore;

use icon_button::IconButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Commit,
    CommitRevision,
    CreateLocalPackage,
    CreateNewRevision,
    Ignore,
    LogInWithBrowser,
    Login,
    Merge,
    Open,
    OpenBrowser,
    OpenInCatalog,
    OpenInFileBrowser,
    Pull,
    Push,
    Remove,
    Reveal,
    Save,
    SetOrigin,
    SetRemote,
    SubmitLogin,
    Unignore,
}

impl ButtonKind {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Commit => "/assets/img/icons/commit.svg",
            Self::CommitRevision => "/assets/img/icons/done.svg",
            Self::CreateLocalPackage => "/assets/img/icons/add.svg",
            Self::CreateNewRevision => "/assets/img/icons/arrow_forward.svg",
            Self::Ignore => "/assets/img/icons/visibility_off.svg",
            Self::LogInWithBrowser => "/assets/img/icons/open_in_browser.svg",
            Self::Login => "/assets/img/icons/warning.svg",
            Self::Merge => "/assets/img/icons/merge.svg",
            Self::Open => "/assets/img/icons/open_in_new.svg",
            Self::OpenBrowser => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInCatalog => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInFileBrowser => "/assets/img/icons/folder_open.svg",
            Self::Pull => "/assets/img/icons/cloud_download.svg",
            Self::Push => "/assets/img/icons/cloud_upload.svg",
            Self::Remove => "/assets/img/icons/block.svg",
            Self::Reveal => "/assets/img/icons/folder_open.svg",
            Self::Save => "/assets/img/icons/done.svg",
            Self::SetOrigin => "/assets/img/icons/warning.svg",
            Self::SetRemote => "/assets/img/icons/cloud_upload.svg",
            Self::SubmitLogin => "/assets/img/icons/done.svg",
            Self::Unignore => "/assets/img/icons/visibility.svg",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Commit => "Commit",
            Self::CommitRevision => "Commit",
            Self::CreateLocalPackage => "Create local package",
            Self::CreateNewRevision => "Create new revision",
            Self::Ignore => "Ignore",
            Self::LogInWithBrowser => "Log in with browser",
            Self::Login => "Login",
            Self::Merge => "Merge",
            Self::Open => "Open",
            Self::OpenBrowser => "Open browser",
            Self::OpenInCatalog => "Open in Catalog",
            Self::OpenInFileBrowser => "Open",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Remove => "Remove",
            Self::Reveal => "Reveal",
            Self::Save => "Save",
            Self::SetOrigin => "Set origin",
            Self::SetRemote => "Set remote",
            Self::SubmitLogin => "Submit code and Log in",
            Self::Unignore => "Ignored",
        }
    }
}

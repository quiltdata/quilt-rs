mod button_cta;
mod icon_button;
pub mod browse;
pub mod change_origin;
pub mod commit;
pub mod commit_revision;
pub mod create_new_revision;
pub mod create_local_package;
pub mod form_primary;
pub mod form_secondary;
pub mod certify_latest;
pub mod collect_logs;
pub mod email_support;
pub mod go_home;
pub mod ignore;
pub mod log_in_with_browser;
pub mod login;
pub mod login_link;
pub mod logout;
pub mod merge;
pub mod open;
pub mod open_browser;
pub mod open_dot_quilt;
pub mod open_in_catalog;
pub mod open_logs_dir;
pub mod open_in_file_browser;
pub mod pull;
pub mod push;
pub mod re_login;
pub mod refresh;
pub mod release_notes;
pub mod reload_page;
pub mod remove;
pub mod reset_local;
pub mod reveal;
pub mod save;
pub mod send_to_sentry;
pub mod settings;
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
pub use form_primary::FormPrimary;
pub use form_secondary::FormSecondary;
pub use certify_latest::CertifyLatest;
pub use collect_logs::CollectLogs;
pub use email_support::EmailSupport;
pub use go_home::GoHome;
pub use ignore::Ignore;
pub use log_in_with_browser::LogInWithBrowser;
pub use login::Login;
pub use login_link::LoginLink;
pub use logout::Logout;
pub use merge::Merge;
pub use open::Open;
pub use open_browser::OpenBrowser;
pub use open_dot_quilt::OpenDotQuilt;
pub use open_in_catalog::OpenInCatalog;
pub use open_logs_dir::OpenLogsDir;
pub use open_in_file_browser::OpenInFileBrowser;
pub use pull::Pull;
pub use push::Push;
pub use re_login::ReLogin;
pub use refresh::Refresh;
pub use release_notes::ReleaseNotes;
pub use reload_page::ReloadPage;
pub use remove::Remove;
pub use reset_local::ResetLocal;
pub use reveal::Reveal;
pub use save::Save;
pub use send_to_sentry::SendToSentry;
pub use settings::Settings;
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
    Logout,
    Merge,
    Open,
    OpenBrowser,
    OpenInCatalog,
    OpenInFileBrowser,
    Pull,
    Push,
    Refresh,
    Remove,
    Reveal,
    Save,
    Settings,
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
            Self::Logout => "/assets/img/icons/warning.svg",
            Self::Merge => "/assets/img/icons/merge.svg",
            Self::Open => "/assets/img/icons/open_in_new.svg",
            Self::OpenBrowser => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInCatalog => "/assets/img/icons/open_in_browser.svg",
            Self::OpenInFileBrowser => "/assets/img/icons/folder_open.svg",
            Self::Pull => "/assets/img/icons/cloud_download.svg",
            Self::Push => "/assets/img/icons/cloud_upload.svg",
            Self::Refresh => "/assets/img/icons/refresh.svg",
            Self::Remove => "/assets/img/icons/block.svg",
            Self::Reveal => "/assets/img/icons/folder_open.svg",
            Self::Save => "/assets/img/icons/done.svg",
            Self::Settings => "/assets/img/icons/gear.svg",
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
            Self::Logout => "Logout",
            Self::Merge => "Merge",
            Self::Open => "Open",
            Self::OpenBrowser => "Open browser",
            Self::OpenInCatalog => "Open in Catalog",
            Self::OpenInFileBrowser => "Open",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Refresh => "Refresh",
            Self::Remove => "Remove",
            Self::Reveal => "Reveal",
            Self::Save => "Save",
            Self::Settings => "Settings",
            Self::SetOrigin => "Set origin",
            Self::SetRemote => "Set remote",
            Self::SubmitLogin => "Submit code and Log in",
            Self::Unignore => "Ignored",
        }
    }
}

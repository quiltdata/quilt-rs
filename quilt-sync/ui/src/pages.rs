mod commit;
mod error;
mod installed_package;
mod installed_package_v2;
mod installed_packages_list;
mod login;
mod main_page;
mod merge;
mod not_found;
mod remote_package;
mod settings;
mod setup;
mod status_watch;

pub use commit::Commit;
pub use error::Error;
pub use installed_package::InstalledPackage;
pub use installed_package_v2::InstalledPackageV2;
// The gallery draws this exact live slice rather than maintaining another
// approximation of the page component.
pub use installed_package_v2::context_pane::{CurrentRevisionPane, RevisionHistoryFetch};
// The gallery's header scene draws the page's own overflow menu, inert.
pub use installed_package_v2::{MenuCommand, MenuItem, menu_items};
// The gallery's live context pane takes the page's wiring; it builds an idle one.
pub use installed_package_v2::Wiring;
// What the file pane will read: the page's one differing set and its scope.
pub use installed_package_v2::keeping::{BacklogDownload, KeepingCommands, ScopeStore};
pub use installed_package_v2::{FileMarks, differing_marks};
// The gallery's resolve cell and scene draw the live pane.
pub use installed_package_v2::resolve::{
    ResolveCommands, ResolvePane, RevisionChoice, differs_sentence,
};
pub use installed_packages_list::InstalledPackagesList;
pub use login::Login;
pub use main_page::MainPage;
// For the gallery, which is a second binary against this library and draws the same
// queue rows. Re-exported one function rather than opening the page's module: a
// second copy of the map in the gallery is a copy that drifts.
pub use main_page::queue::action_href;
pub use merge::Merge;
pub use not_found::NotFound;
pub use remote_package::RemotePackage;
pub use settings::Settings;
pub use setup::Setup;

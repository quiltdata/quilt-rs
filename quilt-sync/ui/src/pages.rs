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
// And the live file pane, over the scene's fixture, beside the scene's drawing.
pub use installed_package_v2::file_pane::{Facet, FileList, FilePane, Grouping, Listing};
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Collect every `.rs` file under `dir`.
    fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a source directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The `g-` classes are gallery chrome, which only `gallery.scss` loads: a
    /// page that borrows one is unstyled in the app, as the file pane's footer
    /// once was. Pages style themselves through their own `.module.scss`.
    #[test]
    fn no_page_uses_a_gallery_only_class() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pages");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let offenders: Vec<String> = files
            .iter()
            .filter(|f| {
                std::fs::read_to_string(f)
                    .expect("a readable source")
                    .contains("class=\"g-")
            })
            .map(|f| f.display().to_string())
            .collect();
        assert!(offenders.is_empty(), "gallery classes in {offenders:?}");
    }
}

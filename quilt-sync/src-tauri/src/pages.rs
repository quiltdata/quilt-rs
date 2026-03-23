use std::path::Path;
use std::path::PathBuf;

use crate::routes::Paths;

mod commit;
mod error;
mod installed_package;
mod installed_packages_list;
mod login;
mod merge;
mod settings;
mod setup;

use crate::app::AppAssets;
use crate::error::Error;
use crate::model::install_package_only;
use crate::model::QuiltModel;
use crate::quilt;
use installed_packages_list::ViewInstalledPackagesList;

pub use commit::ViewCommit;
pub use error::ViewError;
pub use installed_package::ViewInstalledPackage;
pub use login::ViewLogin;
pub use merge::ViewMerge;
pub use settings::ViewSettings;
pub use setup::ViewSetup;

pub async fn load(
    model: &impl QuiltModel,
    app: &impl AppAssets,
    default_home: &Path,
    data_dir: &Path,
    tracing: &crate::telemetry::Telemetry,
    path: &Paths,
) -> Result<String, Error> {
    match path {
        Paths::Commit(namespace) => ViewCommit::create(model, app, tracing, namespace)
            .await?
            .render(),
        Paths::InstalledPackage(namespace) => {
            ViewInstalledPackage::create(model, app, tracing, namespace)
                .await?
                .render()
        }
        Paths::InstalledPackagesList => ViewInstalledPackagesList::create(model, app, tracing)
            .await?
            .render(),
        // location is None here: this path is reached via href buttons
        // (error page "Login", package error status). After login the user
        // returns to InstalledPackagesList, which is the right default.
        // The LoginRequired path in load_page_command passes the real
        // location so redirect-back works for the main auth flow.
        Paths::Login(host) => ViewLogin::create(app, tracing, host.clone(), None)
            .await?
            .render(),
        Paths::LoginError(host, title, error) => {
            ViewError::for_login_error(app, host.clone(), title.clone(), error.clone())
                .await?
                .render()
        }
        Paths::Merge(namespace) => ViewMerge::create(model, app, tracing, namespace)
            .await?
            .render(),
        Paths::RemotePackage(uri) => {
            let installed_package = install_package_only(model, uri).await?;

            // If URI has a path, handle it (for both already-installed and newly-installed packages)
            if let Some(ref path) = uri.path {
                if !model.is_path_installed(&installed_package, path).await? {
                    model
                        .package_install_paths(&installed_package, std::slice::from_ref(path))
                        .await?;
                }
                model
                    .open_in_default_application(&uri.namespace, path)
                    .await?;
            }

            ViewInstalledPackage::create(model, app, tracing, &uri.namespace)
                .await?
                .render()
        }
        Paths::Settings => {
            let data_dir_buf = data_dir.to_path_buf();
            let auth_dir = data_dir_buf.join(quilt::paths::AUTH_DIR);

            let home_dir = model
                .get_quilt()
                .lock()
                .await
                .get_home()
                .await
                .ok()
                .map(|h| PathBuf::from(h.as_ref().clone()));

            let mut auth_hosts: Vec<String> = Vec::new();
            if auth_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&auth_dir) {
                    for entry in entries.flatten() {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            if let Some(name) = entry.file_name().to_str() {
                                auth_hosts.push(name.to_string());
                            }
                        }
                    }
                }
            }
            auth_hosts.sort();

            let log_level = tracing.log_level();

            ViewSettings::create(app, &data_dir_buf, home_dir, log_level, auth_hosts)
                .await?
                .render()
        }
        Paths::Setup => ViewSetup::create(app, default_home).await?.render(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::app::mocks as app_mocks;
    use crate::model::mocks as model_mocks;

    fn default_home() -> PathBuf {
        PathBuf::from("/home/user/QuiltSync")
    }

    fn default_data_dir() -> PathBuf {
        PathBuf::from("/tmp/quiltsync/data")
    }

    fn default_telemetry() -> crate::telemetry::Telemetry {
        crate::telemetry::Telemetry::default()
    }

    #[tokio::test]
    async fn test_commit() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);
        let app = app_mocks::create();

        let url = "https://l/p/commit.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="Commit">Commit"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_package() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);
        let app = app_mocks::create();

        let url = "https://l/p/installed-package.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="foo/bar">foo/bar"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_packages_list(&mut model);
        let app = app_mocks::create();

        let url = "https://l/p/installed-packages-list.html";
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains("any packages"));
        Ok(())
    }

    #[tokio::test]
    async fn test_merge() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);
        let app = app_mocks::create();

        let url = "https://l/p/merge.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="Merge">Merge"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_remote_package() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_remote_package(&mut model);
        let app = app_mocks::create();

        let uri =
            "quilt+s3://quilt-example#package=foo/bar@6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22&path=NAME";
        let url = format!(
            "https://l/p/remote-package.html?uri={}",
            urlencoding::encode(uri)
        );
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains(
            r##"<strong class="qui-breadcrumb-current" title="foo/bar">foo/bar</strong>"##,
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_setup() -> Result<(), Error> {
        let model = model_mocks::create();
        let app = app_mocks::create();

        let url = "https://l/p/setup.html";
        let path: Paths = url.parse()?;
        let page = load(&model, &app, &default_home(), &default_data_dir(), &default_telemetry(), &path).await?;
        assert!(page.contains("Set home directory"));
        Ok(())
    }
}

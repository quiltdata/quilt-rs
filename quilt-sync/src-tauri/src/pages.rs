use std::path::Path;

use crate::routes;
use crate::routes::Paths;

mod commit;
mod error;
mod installed_package;
mod installed_packages_list;
mod login;
mod merge;
mod settings;
mod setup;

use rust_i18n::t;

use crate::app::App;
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
    app: &App,
    default_home: &Path,
    data_dir: &Path,
    tracing: &crate::telemetry::Telemetry,
    path: &Paths,
) -> Result<String, Error> {
    match path {
        Paths::Commit(namespace, filter) => ViewCommit::create(model, tracing, namespace, filter)
            .await?
            .render(),
        Paths::InstalledPackage(namespace, filter) => {
            ViewInstalledPackage::create(model, tracing, namespace, filter)
                .await?
                .render()
        }
        Paths::InstalledPackagesList => ViewInstalledPackagesList::create(model, tracing)
            .await?
            .render(),
        Paths::Login(host, back) => ViewLogin::create(tracing, host.clone(), Some(back.clone()))
            .await?
            .render(),
        Paths::LoginError(host, title, error) => {
            ViewError::for_login_error(host.clone(), title.clone(), error.clone())
                .await?
                .render()
        }
        Paths::Merge(namespace) => ViewMerge::create(model, tracing, namespace).await?.render(),
        Paths::RemotePackage(uri) => match install_package_only(model, uri).await {
            Err(Error::Quilt(quilt::Error::InstallPackage(
                quilt::InstallPackageError::DifferentVersion {
                    requested_hash,
                    installed_hash,
                    ..
                },
            ))) => {
                // Show the installed package page with a notification.
                // The page already renders the appropriate sync button
                // based on UpstreamState (Pull, Push, etc.).
                let short_requested = &requested_hash[..requested_hash.len().min(8)];
                let short_installed = &installed_hash[..installed_hash.len().min(8)];
                let notification = t!(
                    "installed_package_notification.different_version",
                    requested => short_requested,
                    installed => short_installed,
                )
                .to_string();
                ViewInstalledPackage::create(
                    model,
                    tracing,
                    &uri.namespace,
                    &routes::EntriesFilter::for_installed_package(),
                )
                .await?
                .with_notification(notification)
                .render()
            }
            result => {
                let installed_package = result?;

                // If URI has a path, handle it (for both already-installed and newly-installed packages)
                if let Some(ref path) = uri.path {
                    if !model.is_path_installed(&installed_package, path).await? {
                        model
                            .package_install_paths(
                                &installed_package,
                                std::slice::from_ref(path),
                            )
                            .await?;
                    }
                    model
                        .open_in_default_application(&uri.namespace, path)
                        .await?;
                }

                ViewInstalledPackage::create(
                    model,
                    tracing,
                    &uri.namespace,
                    &routes::EntriesFilter::for_installed_package(),
                )
                .await?
                .render()
            }
        },
        Paths::Settings => {
            let data_dir_buf = data_dir.to_path_buf();
            let home_dir = model
                .get_quilt()
                .lock()
                .await
                .get_home()
                .await
                .ok()
                .map(|h| h.as_ref().clone());

            let auth_hosts = quilt::paths::list_auth_hosts(&data_dir_buf);

            let log_level = tracing.log_level();

            ViewSettings::create(app, &data_dir_buf, home_dir, log_level, auth_hosts)
                .await?
                .render()
        }
        Paths::Setup => ViewSetup::create(default_home).await?.render(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::app::App;
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
        let app = App::create()?;

        let url = "https://l/p/commit.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="Commit">Commit"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_package() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);
        let app = App::create()?;

        let url = "https://l/p/installed-package.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="foo/bar">foo/bar"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_packages_list(&mut model);
        let app = App::create()?;

        let url = "https://l/p/installed-packages-list.html";
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains("any packages"));
        Ok(())
    }

    #[tokio::test]
    async fn test_merge() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);
        let app = App::create()?;

        let url = "https://l/p/merge.html#namespace=doesnt/matter";
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains(r#"<strong class="qui-breadcrumb-current" title="Merge">Merge"#));
        Ok(())
    }

    #[tokio::test]
    async fn test_remote_package() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_remote_package(&mut model);
        let app = App::create()?;

        let uri =
            "quilt+s3://quilt-example#package=foo/bar@6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22&path=NAME";
        let url = format!(
            "https://l/p/remote-package.html?uri={}",
            urlencoding::encode(uri)
        );
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains(
            r##"<strong class="qui-breadcrumb-current" title="foo/bar">foo/bar</strong>"##,
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_remote_package_different_version() -> Result<(), Error> {
        let mut model = model_mocks::create();
        model_mocks::mock_remote_package_different_version(&mut model);
        let app = App::create()?;

        let uri = "quilt+s3://quilt-example#package=foo/bar@bbbb2222";
        let url = format!(
            "https://l/p/remote-package.html?uri={}",
            urlencoding::encode(uri)
        );
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;

        // Should show the installed package page (not an error page)
        assert!(page.contains(
            r##"<strong class="qui-breadcrumb-current" title="foo/bar">foo/bar</strong>"##,
        ));
        // Should show the notification with both short hashes
        assert!(page.contains("qui-notification"));
        assert!(page.contains("bbbb2222"));
        assert!(page.contains("aaaa1111"));

        Ok(())
    }

    #[tokio::test]
    async fn test_setup() -> Result<(), Error> {
        let model = model_mocks::create();
        let app = App::create()?;

        let url = "https://l/p/setup.html";
        let path: Paths = url.parse()?;
        let page = load(
            &model,
            &app,
            &default_home(),
            &default_data_dir(),
            &default_telemetry(),
            &path,
        )
        .await?;
        assert!(page.contains("Set home directory"));
        Ok(())
    }
}

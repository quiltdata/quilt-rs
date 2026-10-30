//! Starting the engine: take the data folder's lock, bind the port, build the
//! services, serve.
//!
//! The port is bound before the services start, so a start that fails there
//! leaves nothing running.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use semver::Version;

use crate::engine::Result;
use crate::engine::desktop::Desktop;
use crate::engine::lock;
use crate::engine::lock::Acquired;
use crate::engine::lock::EngineLock;
use crate::engine::server;
use crate::engine::services::Services;
use crate::engine::telemetry::prelude::*;

/// A running engine. Holds the data folder's lock until dropped.
pub struct Engine {
    pub services: Arc<Services>,
    /// The page's address with the token, for a browser to open.
    pub link: String,
    _lock: EngineLock,
}

/// What [`start`] found.
pub enum Started {
    Engine(Engine),
    /// Another process is the engine for this data folder. Its link, when it
    /// has written its address and token.
    Running(Option<String>),
}

/// Start the engine for `data_dir`, serving on loopback with the desktop's
/// port rule.
///
/// # Errors
///
/// When the lock, the port, the token or the services fail.
pub async fn start(
    data_dir: PathBuf,
    version: Version,
    desktop: Option<Arc<dyn Desktop>>,
) -> Result<Started> {
    let lock = match lock::acquire_or_wait_for_info(&data_dir).await? {
        Acquired::Held(lock) => lock,
        Acquired::Running(info) => {
            let link = info.zip(lock::read_token(&data_dir));
            return Ok(Started::Running(
                link.map(|(info, token)| lock::link(&info.url, &token)),
            ));
        }
    };

    let listener = lock::bind_desktop(IpAddr::V4(Ipv4Addr::LOCALHOST), lock.saved_port()).await?;
    let addr = listener.local_addr()?;
    if let Err(err) = lock.save_port(addr.port()) {
        warn!("failed to save the port: {err}");
    }
    let token = lock.web_token()?;

    let services = Services::init(data_dir, version.clone()).await?;
    server::serve(listener, Arc::clone(&services), desktop, token.clone())?;

    let url = format!("http://{addr}");
    lock.write_info(&url, &version.to_string())?;
    Ok(Started::Engine(Engine {
        services,
        link: lock::link(&url, &token),
        _lock: lock,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_start_on_the_folder_finds_the_first() {
        let data_dir = tempfile::tempdir().unwrap();
        let Started::Engine(engine) =
            start(data_dir.path().to_path_buf(), Version::new(1, 2, 3), None)
                .await
                .unwrap()
        else {
            panic!("the first start found another engine");
        };
        assert!(engine.link.starts_with("http://127.0.0.1:"));
        assert!(engine.link.contains("/?t="));

        let Started::Running(link) =
            start(data_dir.path().to_path_buf(), Version::new(1, 2, 3), None)
                .await
                .unwrap()
        else {
            panic!("a second engine started on the same folder");
        };
        assert_eq!(link.as_deref(), Some(engine.link.as_str()));
    }
}

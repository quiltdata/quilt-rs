//! Starting the engine: take the data folder's lock, bind the port, build the
//! services, serve.
//!
//! The port is bound before the services start, so a start that fails there
//! leaves nothing running.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use semver::Version;
use tokio::net::TcpListener;

use crate::engine::Error;
use crate::engine::Result;
use crate::engine::desktop::Desktop;
use crate::engine::lock;
use crate::engine::lock::Acquired;
use crate::engine::lock::EngineLock;
pub use crate::engine::lock::HEADLESS_PORT;
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

/// How to start: which folder, which address, and with what desktop.
pub struct Options {
    pub data_dir: PathBuf,
    pub version: Version,
    /// The desktop app's features; `None` for headless.
    pub desktop: Option<Arc<dyn Desktop>>,
    pub bind: IpAddr,
    pub port: Port,
    /// Make a new token first, so old links stop working.
    pub reset_token: bool,
}

impl Options {
    /// The desktop app's: on loopback, its last port or any.
    pub fn desktop(data_dir: PathBuf, version: Version, desktop: Arc<dyn Desktop>) -> Self {
        Self {
            data_dir,
            version,
            desktop: Some(desktop),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: Port::Desktop,
            reset_token: false,
        }
    }
}

/// Which port to serve on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    /// The last one when free, otherwise any.
    Desktop,
    /// Headless with no `--port`: the last one, else 7878, else the next free.
    Headless,
    /// `--port`: this one or none; `0` takes any.
    Given(u16),
}

/// What [`start`] found.
pub enum Started {
    Engine(Engine),
    /// Another process is the engine for this data folder. Its link, when it
    /// has written its address and token.
    Running(Option<String>),
}

/// Start the engine as `options` say.
///
/// # Errors
///
/// When the lock, the port, the token or the services fail.
pub async fn start(options: Options) -> Result<Started> {
    let Options {
        data_dir,
        version,
        desktop,
        bind,
        port,
        reset_token,
    } = options;
    let lock = match lock::acquire_or_wait_for_info(&data_dir).await? {
        Acquired::Held(lock) => lock,
        Acquired::Running(info) => {
            let link = info.zip(lock::read_token(&data_dir));
            return Ok(Started::Running(
                link.map(|(info, token)| lock::link(&info.url, &token)),
            ));
        }
    };

    let listener = match port {
        Port::Desktop => lock::bind_desktop(bind, lock.saved_port()).await?,
        Port::Headless => lock::bind_headless(bind, lock.saved_port()).await?,
        Port::Given(port) => TcpListener::bind(SocketAddr::new(bind, port))
            .await
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::AddrInUse => Error::PortInUse(port),
                _ => err.into(),
            })?,
    };
    let addr = listener.local_addr()?;
    if let Err(err) = lock.save_port(addr.port()) {
        warn!("failed to save the port: {err}");
    }
    let token = if reset_token {
        lock.new_web_token()?
    } else {
        lock.web_token()?
    };

    let services = Services::init(data_dir, version.clone()).await?;
    // Early, so a page's hello can already say. Tests don't reach the network.
    if cfg!(not(test)) {
        let services = Arc::clone(&services);
        tokio::spawn(async move { services.updates.check().await });
    }
    server::serve(listener, Arc::clone(&services), desktop, token.clone())?;

    let url = format!("http://{}", link_host(addr));
    lock.write_info(&url, &version.to_string())?;
    Ok(Started::Engine(Engine {
        services,
        link: lock::link(&url, &token),
        _lock: lock,
    }))
}

/// The address a browser opens: a loopback or any-address bind is reached on
/// `127.0.0.1`, which the `Host` check allows and browsers treat as secure.
fn link_host(addr: SocketAddr) -> String {
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        format!("127.0.0.1:{}", addr.port())
    } else {
        addr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless(data_dir: &std::path::Path, port: Port) -> Options {
        Options {
            data_dir: data_dir.to_path_buf(),
            version: Version::new(1, 2, 3),
            desktop: None,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            reset_token: false,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_start_on_the_folder_finds_the_first() {
        let data_dir = tempfile::tempdir().unwrap();
        let Started::Engine(engine) = start(headless(data_dir.path(), Port::Given(0)))
            .await
            .unwrap()
        else {
            panic!("the first start found another engine");
        };
        assert!(engine.link.starts_with("http://127.0.0.1:"));
        assert!(engine.link.contains("/?t="));

        let Started::Running(link) = start(headless(data_dir.path(), Port::Given(0)))
            .await
            .unwrap()
        else {
            panic!("a second engine started on the same folder");
        };
        assert_eq!(link.as_deref(), Some(engine.link.as_str()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_given_port_that_is_taken_stops_the_start() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let data_dir = tempfile::tempdir().unwrap();
        let err = start(headless(data_dir.path(), Port::Given(port)))
            .await
            .err()
            .expect("a taken port started");
        assert!(matches!(err, Error::PortInUse(p) if p == port));
        assert!(
            err.to_string().contains("--port 0"),
            "says how to choose another: {err}"
        );
    }
}

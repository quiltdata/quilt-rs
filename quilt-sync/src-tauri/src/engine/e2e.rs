//! Browser tests: headless `QuiltSync` in Chrome, or Firefox, over `WebDriver`,
//! one engine per test on throwaway folders, started in this process.
//!
//! Behind the `e2e` feature, since they need a browser; `just test e2e` builds
//! the UI the server embeds, then runs them. Each test runs in its own process
//! (nextest), so a test may set `HOME` before it starts any threads.

mod restart;

use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use semver::Version;
use thirtyfour::error::WebDriverErrorInner;
use thirtyfour::extensions::query::ElementPollerWithTimeout;
use thirtyfour::prelude::*;
use tokio::runtime::Runtime;
use url::Url;

use crate::engine::commands;
use crate::engine::model::QuiltModel;
use crate::engine::navigation;
use crate::engine::routes;
use crate::engine::services::Services;
use crate::engine::start;
use crate::engine::start::Engine;
use crate::engine::start::Started;

const NAMESPACE: &str = "e2e/first";

/// Where fetched browser drivers are kept, found under the real `HOME` before
/// a test replaces it, so a test reuses the driver the last one fetched.
static DRIVERS: OnceLock<PathBuf> = OnceLock::new();

/// An engine on throwaway folders, which it stops when dropped.
struct QuiltSync {
    /// The data folder.
    domain: PathBuf,
    /// The folder packages live in.
    home: PathBuf,
    running: Option<Running>,
    /// `http://127.0.0.1:<port>/`, with no token.
    origin: Url,
    token: String,
}

/// An engine on a runtime of its own, so stopping it stops everything it
/// spawned, as Ctrl-C does for `--headless`.
struct Running {
    runtime: Runtime,
    engine: Engine,
}

impl QuiltSync {
    /// Start an engine in `base`; `set_up` sets the home folder first, past
    /// Setup.
    async fn start(base: &Path, set_up: bool) -> Self {
        let domain = base.join("domain");
        let running = off_runtime(|| run(&domain));
        let (origin, token) = address(&running.engine);
        let quiltsync = Self {
            domain,
            home: base.join("home"),
            running: Some(running),
            origin,
            token,
        };
        if set_up {
            let home = quiltsync.home.display().to_string();
            quiltsync
                .on_engine(|s| async move { commands::setup(&s, home).await })
                .await
                .expect("setup");
        }
        quiltsync
    }

    /// An address on this engine with the token, e.g. `url("/")`.
    fn url(&self, path: &str) -> String {
        let mut url = self.origin.join(path).expect("a path");
        url.query_pairs_mut().append_pair("t", &self.token);
        url.into()
    }

    /// Run `f` on the engine's runtime, as a command would run.
    async fn on_engine<F, T>(&self, f: impl FnOnce(Arc<Services>) -> F) -> T
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let running = self.running.as_ref().expect("a running engine");
        let services = Arc::clone(&running.engine.services);
        running
            .runtime
            .spawn(f(services))
            .await
            .expect("the engine's task")
    }

    async fn create_package(&self, namespace: &str) {
        let namespace = namespace.to_owned();
        self.on_engine(
            |s| async move { commands::package_create(&s, namespace, None, None).await },
        )
        .await
        .expect("a new package");
    }

    /// Stop the engine, as Ctrl-C does.
    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            off_runtime(|| halt(running));
        }
    }

    /// Start it again on the same folder.
    fn resume(&mut self) {
        assert!(self.running.is_none(), "the engine is still running");
        let running = off_runtime(|| run(&self.domain));
        (self.origin, self.token) = address(&running.engine);
        self.running = Some(running);
    }
}

impl Drop for QuiltSync {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(domain: &Path) -> Running {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let version = Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version");
    let started = runtime
        .block_on(start::start(domain.to_path_buf(), version, None))
        .expect("the engine starts");
    let Started::Engine(engine) = started else {
        panic!("another engine runs on {}", domain.display());
    };
    Running { runtime, engine }
}

/// As `--headless` stops: the lock goes last, once nothing can still write.
fn halt(Running { runtime, engine }: Running) {
    let lock = {
        let _in = runtime.enter();
        engine.services.app.logging.shutdown();
        engine.into_lock()
    };
    runtime.shutdown_timeout(Duration::from_secs(5));
    drop(lock);
}

fn address(engine: &Engine) -> (Url, String) {
    let link = Url::parse(&engine.link).expect("the engine's link");
    let token = link
        .query_pairs()
        .find_map(|(key, value)| (key == "t").then(|| value.into_owned()))
        .expect("a token in the link");
    (link.join("/").expect("the origin"), token)
}

/// Starting and stopping an engine blocks, which a runtime's own thread can't.
fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(f).join().expect("the engine thread"))
}

/// Headless Chrome, or Firefox when `E2E_BROWSER=firefox`, with a driver to
/// match fetched on first use. `CHROME_BIN` or `FIREFOX_BIN` names a browser
/// that isn't where it usually is.
async fn browser() -> WebDriver {
    let caps: Capabilities = match std::env::var("E2E_BROWSER").as_deref() {
        Err(_) | Ok("chrome") => {
            let mut caps = DesiredCapabilities::chrome();
            for arg in ["--headless=new", "--no-sandbox", "--window-size=1280,900"] {
                caps.add_arg(arg).expect("a Chrome argument");
            }
            if let Ok(binary) = std::env::var("CHROME_BIN") {
                caps.set_binary(&binary).expect("a Chrome binary");
            }
            caps.into()
        }
        Ok("firefox") => {
            let mut caps = DesiredCapabilities::firefox();
            for arg in ["-headless", "-width=1280", "-height=900"] {
                caps.add_arg(arg).expect("a Firefox argument");
            }
            if let Ok(binary) = std::env::var("FIREFOX_BIN") {
                caps.set_firefox_binary(&binary).expect("a Firefox binary");
            }
            caps.into()
        }
        Ok(other) => panic!("E2E_BROWSER is chrome or firefox, not {other}"),
    };
    WebDriver::managed(caps)
        .cache_dir(DRIVERS.get().expect("the drivers' folder").clone())
        .poller(Arc::new(ElementPollerWithTimeout::new(
            Duration::from_secs(30),
            Duration::from_millis(100),
        )))
        .await
        .expect("Chrome over WebDriver")
}

/// The test's body, on a runtime the browser sessions use. `body` gets a
/// throwaway folder to start the engine in; its `user` folder is `HOME`.
fn test<F: Future<Output = WebDriverResult<()>>>(body: impl FnOnce(PathBuf) -> F) {
    let root = tempfile::tempdir().expect("a temp folder");
    // The engine reports resolved paths, and macOS's temp folder is a link.
    let base = std::fs::canonicalize(root.path()).expect("the temp folder");
    let user = base.join("user");
    std::fs::create_dir(&user).expect("a home for the user");
    DRIVERS.get_or_init(|| {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("thirtyfour")
            .join("drivers")
    });
    // SAFETY: each test is its own process, and no thread is started yet.
    unsafe { std::env::set_var("HOME", &user) };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(body(base))
        .expect("the test's browser calls");
}

/// An element whose own text is `text`.
fn text(text: &str) -> By {
    By::XPath(format!("//*[normalize-space(text())='{text}']"))
}

/// An element whose text, its children's included, has `text` in it.
fn containing(tag: &str, text: &str) -> By {
    By::XPath(format!("//{tag}[contains(normalize-space(.), '{text}')]"))
}

fn button(name: &str) -> By {
    By::XPath(format!("//button[normalize-space(.)='{name}']"))
}

/// Click the element `by` finds once it can be clicked, finding it again if
/// the page drew it anew in between, as a list does when it refreshes, or
/// hadn't laid it out yet, which Firefox reports as not interactable.
async fn click(driver: &WebDriver, by: By) -> WebDriverResult<()> {
    for _ in 0..20 {
        let clicked = match driver.query(by.clone()).and_clickable().first().await {
            Ok(element) => element.click().await,
            Err(err) => Err(err),
        };
        match clicked {
            Err(err)
                if matches!(
                    *err,
                    WebDriverErrorInner::StaleElementReference(_)
                        | WebDriverErrorInner::ElementNotInteractable(_)
                ) =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            done => return done,
        }
    }
    panic!("{by:?} could not be clicked");
}

/// Wait until the page's title is `title`.
async fn wait_for_title(driver: &WebDriver, title: &str) -> WebDriverResult<()> {
    driver
        .query(By::XPath(format!("//title[normalize-space(.)='{title}']")))
        .first()
        .await
        .map(drop)
}

/// Mark the page, to tell later that it never reloaded.
async fn mark(driver: &WebDriver) -> WebDriverResult<()> {
    driver
        .execute("window.e2eMark = true;", vec![])
        .await
        .map(drop)
}

async fn still_marked(driver: &WebDriver) -> WebDriverResult<bool> {
    driver
        .execute("return 'e2eMark' in window;", vec![])
        .await?
        .convert()
}

/// Write `name` into the package's folder until every page lists it, with
/// no reload: the watcher's event reaches the pages over the socket.
///
/// The watcher picks up a new package on its next reconcile, a few seconds
/// after it is made, and a write before that goes unseen; writing again until
/// the file shows covers that without knowing how long it is.
async fn write_until_listed(
    quiltsync: &QuiltSync,
    name: &str,
    drivers: &[&WebDriver],
) -> WebDriverResult<()> {
    let folder = quiltsync
        .on_engine(|s| async move {
            let namespace = quilt_uri::Namespace::try_from(NAMESPACE).map_err(|e| e.to_string())?;
            s.model
                .package_home(&namespace)
                .await
                .map(|home| home.display().to_string())
                .map_err(|e| e.to_string())
        })
        .await
        .expect("the package's folder");
    // The list is drawn before the write, so a row can only come from the
    // watcher's event.
    for driver in drivers {
        driver
            .query(text("This package has no files yet."))
            .first()
            .await?;
        mark(driver).await?;
    }
    let file = text(name);
    'rounds: for round in 1..=8 {
        std::fs::write(Path::new(&folder).join(name), format!("round {round}\n"))?;
        for driver in drivers {
            let listed = driver
                .query(file.clone())
                .wait(Duration::from_secs(4), Duration::from_millis(100))
                .exists()
                .await?;
            if !listed {
                continue 'rounds;
            }
        }
        for driver in drivers {
            assert!(still_marked(driver).await?, "the page reloaded");
        }
        return Ok(());
    }
    panic!("{name} never showed on every page");
}

#[test]
fn setup_takes_a_typed_folder_and_opens_the_main_page() {
    test(|base| async move {
        let quiltsync = QuiltSync::start(&base, false).await;
        let driver = browser().await;
        driver.goto(quiltsync.url("/")).await?;
        driver
            .query(containing("h1", "Set up QuiltSync"))
            .first()
            .await?;
        // The link's token moved into the page and left the address.
        let address = driver.current_url().await?;
        assert!(
            address.query_pairs().all(|(key, _)| key != "t"),
            "{address}"
        );

        // Headless has no folder picker: the path is typed.
        let field = driver
            .query(By::XPath(
                "//input[@id=//label[normalize-space(.)='Set home directory']/@for]",
            ))
            .first()
            .await?;
        // The field starts with the default folder.
        field.clear().await?;
        field
            .send_keys(quiltsync.home.display().to_string())
            .await?;
        driver
            .query(text("QuiltSync will make this folder."))
            .first()
            .await?;
        click(&driver, button("Save")).await?;

        driver.query(text("No packages yet")).first().await?;
        wait_for_title(&driver, "QuiltSync").await?;
        assert!(quiltsync.home.is_dir(), "setup made the typed folder");
        driver.quit().await
    });
}

#[test]
fn a_package_made_in_the_dialog_opens_and_a_new_file_appears_on_its_page() {
    test(|base| async move {
        let quiltsync = QuiltSync::start(&base, true).await;
        let driver = browser().await;
        driver.goto(quiltsync.url("/")).await?;
        click(&driver, button("Create package")).await?;
        driver
            .query(By::XPath(
                "//dialog[@open]//input[@placeholder='owner/package-name']",
            ))
            .first()
            .await?
            .send_keys(NAMESPACE)
            .await?;
        click(
            &driver,
            By::XPath("//dialog[@open]//button[normalize-space(.)='Create']"),
        )
        .await?;
        driver
            .query(By::XPath("//dialog[@open]"))
            .wait_until_gone()
            .await?;

        // Right after Create: nothing in the way of the new row.
        click(&driver, containing("a", NAMESPACE)).await?;
        wait_for_title(&driver, &format!("{NAMESPACE} · QuiltSync")).await?;

        write_until_listed(&quiltsync, "hello.txt", &[&driver]).await?;
        driver.quit().await
    });
}

#[test]
fn two_tabs_on_one_package_both_see_a_new_file() {
    test(|base| async move {
        let quiltsync = QuiltSync::start(&base, true).await;
        quiltsync.create_package(NAMESPACE).await;
        let page = quiltsync.url(&format!("/installed-package?namespace={NAMESPACE}"));

        // Two browsers: two sessions, each with its own socket.
        let drivers = [browser().await, browser().await];
        for driver in &drivers {
            driver.goto(&page).await?;
            wait_for_title(driver, &format!("{NAMESPACE} · QuiltSync")).await?;
        }

        let [first, second] = &drivers;
        write_until_listed(&quiltsync, "shared.txt", &[first, second]).await?;
        for driver in drivers {
            driver.quit().await?;
        }
        Ok(())
    });
}

/// The tray and a notice open the page that fixes something. On that page
/// already, the window stays as it is: a reload would blank it while the UI
/// loads again.
#[test]
fn a_window_sent_to_the_page_it_is_on_stays_without_a_reload() {
    test(|base| async move {
        let quiltsync = QuiltSync::start(&base, true).await;
        let driver = browser().await;
        driver
            .goto(format!("{}&window", quiltsync.url("/")))
            .await?;
        driver.query(text("No packages yet")).first().await?;
        mark(&driver).await?;

        let followed = quiltsync
            .on_engine(|s| async move {
                s.navigations
                    .send(&s.events, navigation::WINDOW, routes::Paths::Home);
                // Followed once the engine stops holding it.
                tokio::time::timeout(Duration::from_secs(30), async {
                    while s.navigations.pending(navigation::WINDOW).is_some() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await
            })
            .await;
        assert!(followed.is_ok(), "the page never said it followed");
        driver.query(text("No packages yet")).first().await?;
        assert!(still_marked(&driver).await?, "the page reloaded");
        driver.quit().await
    });
}

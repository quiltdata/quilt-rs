//! A fake engine for the DOM tests: it opens no socket and answers what the
//! test tells it to, so a test can drive the connection through every turn.
//!
//! Installing one starts the connection over; dropping it leaves the page with
//! no way to the engine again, as tests have by default.

use std::cell::Cell;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde_json::Value;
use serde_json::json;

use super::link;
use super::link::Connector;
use super::link::Socket;

#[derive(Default)]
struct Inner {
    /// Every socket opened, by id.
    opened: RefCell<Vec<u64>>,
    /// Every message sent, with its socket's id.
    sent: RefCell<Vec<(u64, Value)>>,
    /// What the engine answers when asked whether it refuses the token.
    refuses: Cell<bool>,
    delays: RefCell<Vec<i32>>,
    reloads: RefCell<Vec<String>>,
    /// Says hello on each socket and answers every request with `null`.
    serving: Cell<bool>,
    /// Says it has no desktop, as headless does.
    headless: Cell<bool>,
}

pub(crate) struct FakeEngine {
    inner: Rc<Inner>,
}

struct FakeConnector(Rc<Inner>);

struct FakeSocket {
    id: u64,
    inner: Rc<Inner>,
}

impl Socket for FakeSocket {
    fn send(&self, text: &str) -> Result<(), String> {
        let message: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if self.inner.serving.get()
            && let Some(request) = message.get("id").cloned()
        {
            let id = self.id;
            wasm_bindgen_futures::spawn_local(async move {
                link::received(id, &json!({ "id": request, "ok": null }).to_string());
            });
        }
        self.inner.sent.borrow_mut().push((self.id, message));
        Ok(())
    }
}

impl Connector for FakeConnector {
    fn open(&self, id: u64) -> Result<Box<dyn Socket>, String> {
        self.0.opened.borrow_mut().push(id);
        if self.0.serving.get() {
            wasm_bindgen_futures::spawn_local({
                let desktop = !self.0.headless.get();
                async move { link::received(id, &hello(link::VERSION, desktop)) }
            });
        }
        Ok(Box::new(FakeSocket {
            id,
            inner: Rc::clone(&self.0),
        }))
    }

    fn refused(&self) -> Pin<Box<dyn Future<Output = Option<&'static str>>>> {
        let message = self.0.refuses.get().then_some(if self.0.headless.get() {
            link::REFUSED_HEADLESS
        } else {
            link::REFUSED
        });
        Box::pin(async move { message })
    }

    fn after(&self, ms: i32, then: Box<dyn FnOnce()>) {
        self.0.delays.borrow_mut().push(ms);
        wasm_bindgen_futures::spawn_local(async move { then() });
    }

    fn reload(&self, version: &str) -> bool {
        let mut reloads = self.0.reloads.borrow_mut();
        let first = !reloads.iter().any(|v| v == version);
        reloads.push(version.to_owned());
        first
    }
}

fn hello(version: &str, desktop: bool) -> String {
    json!({ "hello": { "version": version, "desktop": desktop, "updateAvailable": null } })
        .to_string()
}

impl FakeEngine {
    /// A fake the test drives by hand.
    pub(crate) fn install() -> Self {
        let inner = Rc::new(Inner::default());
        link::install(Some(Rc::new(FakeConnector(Rc::clone(&inner)))));
        Self { inner }
    }

    /// A fake that says hello and answers `null` on its own.
    pub(crate) fn serving() -> Self {
        let engine = Self::install();
        engine.inner.serving.set(true);
        engine
    }

    fn socket(&self) -> u64 {
        *self
            .inner
            .opened
            .borrow()
            .last()
            .expect("a socket was opened")
    }

    pub(crate) fn sockets_opened(&self) -> usize {
        self.inner.opened.borrow().len()
    }

    pub(crate) fn hello(&self) {
        self.hello_from(link::VERSION);
    }

    pub(crate) fn hello_from(&self, version: &str) {
        link::received(self.socket(), &hello(version, !self.inner.headless.get()));
    }

    pub(crate) fn event(&self, name: &str, payload: &Value) {
        link::received(
            self.socket(),
            &json!({ "event": name, "payload": payload }).to_string(),
        );
    }

    pub(crate) fn reply(&self, id: &Value, ok: &Value) {
        link::received(self.socket(), &json!({ "id": id, "ok": ok }).to_string());
    }

    pub(crate) fn close(&self) {
        link::closed(self.socket());
    }

    /// From now on the engine says it has no desktop, as headless does.
    pub(crate) fn as_headless(&self) {
        self.inner.headless.set(true);
    }

    pub(crate) fn refuse_token(&self) {
        self.inner.refuses.set(true);
    }

    /// The requests sent on the socket now open, oldest first.
    pub(crate) fn requests(&self) -> Vec<Value> {
        let socket = self.socket();
        self.inner
            .sent
            .borrow()
            .iter()
            .filter(|(id, _)| *id == socket)
            .map(|(_, message)| message.clone())
            .collect()
    }

    /// Every request sent, on any socket.
    pub(crate) fn all_requests(&self) -> Vec<Value> {
        self.inner
            .sent
            .borrow()
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }

    pub(crate) fn delays(&self) -> Vec<i32> {
        self.inner.delays.borrow().clone()
    }

    pub(crate) fn reloads(&self) -> Vec<String> {
        self.inner.reloads.borrow().clone()
    }
}

/// Send `cmd` now, and keep its answer for the test to read once it lands.
pub(crate) fn ask(cmd: &'static str) -> Rc<RefCell<Option<Result<Value, String>>>> {
    let answer = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&answer);
    wasm_bindgen_futures::spawn_local(async move {
        let got = super::invoke_unit::<Value>(cmd).await;
        *slot.borrow_mut() = Some(got);
    });
    answer
}

impl Drop for FakeEngine {
    fn drop(&mut self) {
        link::install(None);
    }
}

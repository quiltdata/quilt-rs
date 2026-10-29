//! The backend's event bus, free of Tauri.
//!
//! Everything the engine and the commands announce goes into one broadcast
//! channel as `(name, payload)`. The desktop app forwards each event to the
//! window under the same name.

use std::future::Future;
use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::engine::telemetry::prelude::*;

/// How many events a slow subscriber may fall behind before it starts missing
/// them.
const CAPACITY: usize = 1024;

pub type Event = (String, serde_json::Value);

#[derive(Clone)]
pub struct EventSink {
    tx: broadcast::Sender<Event>,
}

impl Default for EventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(CAPACITY);
        Self { tx }
    }

    /// Announce `name` with `payload`. Nobody listening is not an error.
    pub fn emit(&self, name: &str, payload: impl Serialize) {
        match serde_json::to_value(payload) {
            Ok(value) => {
                // `Err` only means there is no subscriber right now.
                let _ = self.tx.send((name.to_owned(), value));
            }
            Err(err) => warn!("events: failed to serialize {name}: {err}"),
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Remember the runtime the services run on, for [`spawn`] calls from threads
/// outside it (the file watcher's debouncer thread, for one).
pub fn set_runtime(handle: tokio::runtime::Handle) {
    let _ = RUNTIME.set(handle);
}

/// Spawn on the current runtime, or on the services' runtime from a plain
/// thread.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.spawn(future),
        Err(_) => RUNTIME
            .get()
            .expect("services runtime is set before any spawn from a plain thread")
            .spawn(future),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_subscriber_gets_name_and_json_payload() {
        let sink = EventSink::new();
        let mut rx = sink.subscribe();
        sink.emit("toast", serde_json::json!({"id": 1}));
        sink.emit("autosync-changed", ());
        assert_eq!(
            rx.recv().await.unwrap(),
            ("toast".to_owned(), serde_json::json!({"id": 1}))
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            ("autosync-changed".to_owned(), serde_json::Value::Null)
        );
    }
}

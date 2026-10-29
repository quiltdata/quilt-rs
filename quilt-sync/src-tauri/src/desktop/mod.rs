//! The desktop app: the Tauri window, its commands, the tray and deep links.

use tauri::Emitter;
use tokio::sync::broadcast::error::RecvError;

use crate::engine::events::EventSink;
use crate::engine::telemetry::prelude::*;

pub mod commands;
pub mod tray;
pub mod uri;

/// Send every engine event to the window under its own name.
pub fn forward_events<R: tauri::Runtime>(app: &tauri::AppHandle<R>, events: &EventSink) {
    let app = app.clone();
    let mut rx = events.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok((name, payload)) => {
                    if let Err(err) = app.emit(&name, payload) {
                        warn!("failed to emit {name}: {err}");
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    warn!("the window missed {missed} events");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use tauri::Listener;

    use super::*;

    #[test]
    fn an_engine_event_reaches_the_window_under_its_name() {
        let app = tauri::test::mock_app();
        let events = EventSink::new();
        let (tx, rx) = mpsc::channel();
        app.listen_any("toast", move |event| {
            let _ = tx.send(event.payload().to_owned());
        });

        forward_events(app.handle(), &events);
        events.emit("toast", serde_json::json!({"id": 1}));

        let payload = rx.recv_timeout(Duration::from_secs(5)).expect("forwarded");
        assert_eq!(payload, r#"{"id":1}"#);
    }
}

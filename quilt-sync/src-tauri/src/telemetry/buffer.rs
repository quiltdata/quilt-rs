//! Events that could not be delivered, kept on disk until they can be.
//!
//! A desktop app is offline routinely — a plane, a hospital VPN, a hotel captive
//! portal — and that is a population worth counting rather than losing. The queue
//! upstream of this is memory, so without it an offline session reports nothing at
//! all and looks identical to nobody using the app.
//!
//! **Durability is incremental, not a shutdown hook.** Nothing runs when this app
//! exits: the framework's run call ends the process directly, so no destructor and
//! no flush gets a chance. A buffer written on the way out would never be written,
//! so it is written the moment a send fails instead.
//!
//! What is stored is the **wire form** — the event exactly as the ingest API would
//! have received it. That is what makes a replay indistinguishable from a fresh
//! send: the payload already carries the timestamp of the moment the user acted and
//! the idempotency key the API dedupes on, both stamped at emission. Storing the
//! typed event instead would re-derive them at replay and date every recovered
//! event to the moment the network came back.

use std::path::{Path, PathBuf};

use mixpanel_rs::Event;

use crate::telemetry::prelude::*;

/// The file, in the app's own data directory beside `install_id`,
/// `publish_settings.json` and `logs/` — the convention every other persisted
/// concern here follows.
///
/// One event per line, so keeping and reading are appends and splits rather than a
/// parse of the whole. A torn write costs the line it was writing, not the file.
const FILE_NAME: &str = "unsent_events.jsonl";

/// How many events may wait for a network before the oldest are dropped.
///
/// Something must go, and it is the oldest: an event's value decays, and a long
/// offline session is more usefully represented by its recent shape than by its
/// first minute. Sized so a genuinely long offline session survives whole — at this
/// app's human-paced volumes that is days of use — while a permanently offline
/// install cannot grow a file without end.
const MAX_BUFFERED: usize = 1000;

/// Where undelivered events wait.
///
/// A path and nothing else: the buffer holds no state of its own, so a failure to
/// read it is answered by treating it as empty rather than by keeping a flag that
/// could disagree with the disk.
#[derive(Clone)]
pub struct Buffer {
    path: PathBuf,
}

impl Buffer {
    pub fn new(base: &Path) -> Self {
        Self {
            path: base.join(FILE_NAME),
        }
    }

    /// Keep `events` for a later attempt, oldest discarded past the bound.
    ///
    /// Read-modify-write rather than an append, because the bound has to be enforced
    /// somewhere and this runs only when a send has already failed — a moment that
    /// is rare and already slow. An append plus a separate trim would leave the file
    /// unbounded in exactly the case the bound exists for: a session that is offline
    /// for its whole life.
    pub fn keep(&self, events: &[Event]) {
        let mut lines = self.read_lines();
        for event in events {
            match serde_json::to_string(event) {
                Ok(line) => lines.push(line),
                // The event is lost, and that is the honest outcome: it cannot be
                // sent either, since sending serializes the same value.
                Err(err) => warn!("telemetry: undeliverable event could not be kept: {err}"),
            }
        }

        if lines.len() > MAX_BUFFERED {
            let dropped = lines.len() - MAX_BUFFERED;
            warn!("telemetry: buffer full, dropped {dropped} of the oldest unsent events");
            lines.drain(..dropped);
        }

        self.write_lines(&lines);
    }

    /// Read everything kept, **leaving it on disk**.
    ///
    /// Non-destructive on purpose, and an earlier revision of this had it the other
    /// way round on the reasoning that a caller must not be able to read the same
    /// events twice. That reasoning ignored the interruption that matters: nothing
    /// runs when this app exits, so between a destructive read and a delivery there
    /// is a window in which the only copy is a local `Vec` — and a quit inside it
    /// loses the whole buffer, including sessions' worth of events that had already
    /// survived being offline.
    ///
    /// So the file is authoritative until a delivery is *confirmed*, and
    /// [`Self::forget_delivered`] is what shortens it. The cost is the mirror-image
    /// risk — delivered, then interrupted before the file is trimmed, so the events
    /// are sent again — and that one is already paid for: every event carries the
    /// idempotency key the ingest API dedupes on, which exists for exactly this.
    /// Losing events is permanent; sending one twice is not.
    pub fn load(&self) -> Vec<Event> {
        let lines = self.read_lines();
        if lines.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::with_capacity(lines.len());
        let mut unreadable = 0usize;
        for line in lines {
            match serde_json::from_str(&line) {
                Ok(event) => events.push(event),
                Err(_) => unreadable += 1,
            }
        }

        if unreadable > 0 {
            // Per-line storage is what makes this a partial loss rather than a total
            // one: a torn write or a half-flushed page costs its own line.
            warn!("telemetry: skipped {unreadable} unreadable buffered events");
        }

        events
    }

    /// Drop the first `count` events, having seen them delivered.
    ///
    /// A prefix rather than a set, because delivery walks the file in order — so this
    /// needs no identity matching and cannot half-remove a batch.
    pub fn forget_delivered(&self, count: usize) {
        if count == 0 {
            return;
        }
        let lines = self.read_lines();
        let remaining = lines.get(count..).unwrap_or_default();
        self.write_lines(remaining);
    }

    fn read_lines(&self) -> Vec<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            // A missing file is the normal case — every install that has never been
            // offline — so it is not worth a word in the log.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                warn!("telemetry: buffer unreadable, treating it as empty: {err}");
                Vec::new()
            }
        }
    }

    /// Replace the file's contents, via a temporary file and a rename.
    ///
    /// Atomic for the same reason the install identity is: a torn buffer is not a
    /// buffer with one bad line but a file whose *tail* is arbitrary, and this write
    /// happens while the app is already having a bad time.
    fn write_lines(&self, lines: &[String]) {
        if lines.is_empty() {
            let _ = std::fs::remove_file(&self.path);
            return;
        }

        let temp = self.path.with_extension("jsonl.tmp");
        let body = lines.join("\n") + "\n";
        if let Err(err) = std::fs::write(&temp, body) {
            warn!("telemetry: could not write the buffer: {err}");
            return;
        }
        if let Err(err) = std::fs::rename(&temp, &self.path) {
            warn!("telemetry: could not replace the buffer: {err}");
            let _ = std::fs::remove_file(&temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn event(name: &str) -> Event {
        Event {
            event: name.to_owned(),
            properties: HashMap::from([(
                "$insert_id".to_owned(),
                serde_json::Value::String(format!("id-{name}")),
            )]),
        }
    }

    fn names(events: &[Event]) -> Vec<String> {
        events.iter().map(|e| e.event.clone()).collect()
    }

    /// The whole point: what was kept comes back, in order, with its properties —
    /// including the timestamp and idempotency key that make a replay honest.
    #[test]
    fn what_is_kept_comes_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());

        buffer.keep(&[event("first"), event("second")]);
        let taken = buffer.load();

        assert_eq!(names(&taken), vec!["first", "second"]);
        assert_eq!(
            taken[0].properties.get("$insert_id"),
            Some(&serde_json::Value::String("id-first".to_owned())),
            "the idempotency key did not survive, so a replay would double-count"
        );
    }

    /// **Reading does not empty it.** The regression a reviewer caught: with a
    /// destructive read, the only copy of the events between reading and delivering
    /// is a local `Vec` — and since nothing runs when this app exits, a quit in that
    /// window loses the whole buffer, sessions of it. The file stays authoritative
    /// until a delivery is confirmed.
    #[test]
    fn reading_leaves_the_events_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());
        buffer.keep(&[event("only")]);

        assert_eq!(buffer.load().len(), 1);
        assert_eq!(
            buffer.load().len(),
            1,
            "an interrupted drain would have lost these"
        );
    }

    /// Confirming delivery is what shortens the file, and only by what was delivered.
    #[test]
    fn forgetting_drops_only_the_delivered_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());
        buffer.keep(&[event("first"), event("second"), event("third")]);

        buffer.forget_delivered(2);

        assert_eq!(names(&buffer.load()), vec!["third"]);
    }

    /// Forgetting everything leaves nothing, so an install that recovers is clean.
    #[test]
    fn forgetting_all_of_them_empties_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());
        buffer.keep(&[event("first"), event("second")]);

        buffer.forget_delivered(2);

        assert!(buffer.load().is_empty());
        assert!(!buffer.path.exists(), "an empty buffer left a file behind");
    }

    /// Keeping accumulates across failures rather than replacing — an offline
    /// session fails many times and each attempt must add to the pile.
    #[test]
    fn keeping_accumulates_across_failures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());

        buffer.keep(&[event("first")]);
        buffer.keep(&[event("second")]);

        assert_eq!(names(&buffer.load()), vec!["first", "second"]);
    }

    /// Past the bound the oldest go, and the newest are what remain.
    #[test]
    fn the_oldest_are_dropped_at_the_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());

        let events: Vec<Event> = (0..MAX_BUFFERED + 10)
            .map(|i| event(&i.to_string()))
            .collect();
        buffer.keep(&events);

        let taken = buffer.load();
        assert_eq!(taken.len(), MAX_BUFFERED);
        assert_eq!(
            taken.first().map(|e| e.event.as_str()),
            Some("10"),
            "the wrong end was dropped"
        );
        assert_eq!(
            taken.last().map(|e| e.event.as_str()),
            Some((MAX_BUFFERED + 9).to_string().as_str())
        );
    }

    /// One unreadable line costs its own line and nothing else. This is the reason
    /// for a line per event rather than one document.
    #[test]
    fn an_unreadable_line_does_not_cost_the_others() {
        let dir = tempfile::tempdir().expect("tempdir");
        let buffer = Buffer::new(dir.path());
        buffer.keep(&[event("kept")]);

        let contents = std::fs::read_to_string(&buffer.path).expect("written");
        std::fs::write(&buffer.path, format!("{{ not json\n{contents}")).expect("write");

        assert_eq!(names(&buffer.load()), vec!["kept"]);
    }

    /// An install that has never been offline has no file, which is not an error.
    #[test]
    fn a_missing_buffer_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(Buffer::new(dir.path()).load().is_empty());
    }
}

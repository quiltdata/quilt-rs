//! The revision popover's state: which opening of which package an answer
//! belongs to, and what the surface draws meanwhile.
//!
//! Pure, so the keying is proven by host tests. An answer lands only on the
//! session and namespace that asked for it; closing, retrying or another
//! package's answer arriving late cannot paint stale rows.

use crate::commands;

/// Which opening of which package's popover an answer belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    namespace: String,
    session: u64,
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Shown {
    Loading,
    Rows(Vec<commands::RevisionHistoryRow>),
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct History {
    namespace: String,
    session: u64,
    shown: Shown,
}

impl History {
    pub(super) fn new(namespace: String) -> Self {
        Self {
            namespace,
            session: 0,
            shown: Shown::Loading,
        }
    }

    /// A new session, drawn as loading; returns the key its fetch must present.
    pub(super) fn open(&mut self) -> Key {
        self.session += 1;
        self.shown = Shown::Loading;
        Key {
            namespace: self.namespace.clone(),
            session: self.session,
        }
    }

    /// Same as `open`: a retry is a new session, so the failed one's late
    /// answer cannot land on top of it.
    pub(super) fn retry(&mut self) -> Key {
        self.open()
    }

    /// Ends the session. Anything still in flight is dropped when it lands.
    pub(super) fn close(&mut self) {
        // Advancing here too keeps open → close → open at three distinct sessions.
        self.session += 1;
        self.shown = Shown::Loading;
    }

    /// Apply an answer if `key` is the current session of this namespace.
    /// Returns whether it was applied.
    pub(super) fn settle(
        &mut self,
        key: &Key,
        answer: Result<Vec<commands::RevisionHistoryRow>, String>,
    ) -> bool {
        let current = key.namespace == self.namespace
            && key.session == self.session
            && self.shown == Shown::Loading;
        if current {
            self.shown = match answer {
                Ok(rows) => Shown::Rows(rows),
                Err(_) => Shown::Failed,
            };
        }
        current
    }

    pub(super) fn shown(&self) -> &Shown {
        &self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::{History, Shown};
    use crate::commands::RevisionHistoryRow;

    fn rows() -> Vec<RevisionHistoryRow> {
        vec![RevisionHistoryRow {
            message: Some("Initial upload".to_string()),
            obtained_at: 1_758_500_000_000.0,
            published: true,
            catalog_url: None,
        }]
    }

    fn history() -> History {
        History::new("team/dataset".to_string())
    }

    #[test]
    fn an_answer_for_the_current_opening_is_drawn() {
        let mut history = history();
        let key = history.open();
        assert!(history.settle(&key, Ok(rows())));
        assert_eq!(history.shown(), &Shown::Rows(rows()));
    }

    #[test]
    fn a_refusal_is_drawn_as_the_failure() {
        let mut history = history();
        let key = history.open();
        assert!(history.settle(&key, Err("AccessDenied".to_string())));
        assert_eq!(history.shown(), &Shown::Failed);
    }

    #[test]
    fn a_retry_drops_the_failed_session_s_late_answer() {
        let mut history = history();
        let failed = history.open();
        assert!(history.settle(&failed, Err("AccessDenied".to_string())));
        let retried = history.retry();

        assert!(!history.settle(&failed, Ok(rows())));
        assert_eq!(history.shown(), &Shown::Loading);

        assert!(history.settle(&retried, Ok(rows())));
        assert_eq!(history.shown(), &Shown::Rows(rows()));
    }

    #[test]
    fn closing_drops_what_is_in_flight() {
        let mut history = history();
        let first = history.open();
        history.close();
        let _second = history.open();

        assert!(!history.settle(&first, Ok(rows())));
        assert_eq!(history.shown(), &Shown::Loading);
    }

    #[test]
    fn another_package_s_answer_is_dropped() {
        let mut other = History::new("team/other".to_string());
        let mut history = history();
        let foreign = other.open();
        history.open();

        assert!(!history.settle(&foreign, Ok(rows())));
        assert_eq!(history.shown(), &Shown::Loading);
    }

    /// The list is lazy per open, so a reopening re-fetches rather than
    /// showing the last list.
    #[test]
    fn every_opening_starts_from_the_skeleton() {
        let mut history = history();
        let key = history.open();
        assert!(history.settle(&key, Ok(rows())));
        history.close();
        history.open();

        assert_eq!(history.shown(), &Shown::Loading);
    }
}

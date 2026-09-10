//! Whether a quit may proceed, or has to ask first.
//!
//! Quitting is the only way an in-flight apply gets interrupted that a person
//! chooses — every other way (an S3 failure, an expiring credential, a kill,
//! power loss) is not a decision anyone makes in a dialog's worth of time. So
//! it is the only one worth asking about, and this is where the asking is
//! decided.
//!
//! Deliberately free of Tauri types: the decision and the pending state are
//! testable on their own, and the window handling stays in [`crate::tray`].

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// What a quit request should do.
#[derive(Debug, PartialEq, Eq)]
pub enum QuitAction {
    /// Nothing is being written — go.
    Exit,
    /// An apply is writing working files: ask before interrupting it.
    Prompt,
    /// A prompt is already up. A second click on Quit must not raise a second
    /// one, and must not exit behind the first even if the apply has since
    /// finished — the user is being asked, and the answer decides.
    AlreadyPrompting,
}

/// The pending-quit state behind the prompt.
#[derive(Default)]
pub struct QuitGate {
    /// A quit was requested and deferred to the prompt.
    pending: AtomicBool,
    /// The window reported the prompt is on screen. Until then the quit is
    /// unanswerable — nobody has been asked — which is what separates a wedged
    /// webview from a user who is still deciding.
    shown: AtomicBool,
}

impl QuitGate {
    #[must_use]
    pub fn request(&self, apply_in_progress: bool) -> QuitAction {
        if self.pending.load(Ordering::SeqCst) {
            return QuitAction::AlreadyPrompting;
        }
        if !apply_in_progress {
            return QuitAction::Exit;
        }
        self.pending.store(true, Ordering::SeqCst);
        QuitAction::Prompt
    }

    /// The window says the prompt is up.
    pub fn note_shown(&self) {
        self.shown.store(true, Ordering::SeqCst);
    }

    /// The user chose to stay.
    pub fn cancel(&self) {
        self.pending.store(false, Ordering::SeqCst);
        self.shown.store(false, Ordering::SeqCst);
    }

    /// A quit is deferred and **nobody was asked** — the prompt never came up,
    /// so no answer is coming and the quit must proceed rather than hang. Read
    /// once after a grace period, never as a running poll.
    #[must_use]
    pub fn unanswerable(&self) -> bool {
        self.pending.load(Ordering::SeqCst) && !self.shown.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_with_nothing_applying_exits() {
        let gate = QuitGate::default();
        assert_eq!(gate.request(false), QuitAction::Exit);
        assert!(
            !gate.unanswerable(),
            "an exit leaves nothing pending to answer"
        );
    }

    #[test]
    fn a_quit_during_an_apply_prompts() {
        let gate = QuitGate::default();
        assert_eq!(gate.request(true), QuitAction::Prompt);
    }

    #[test]
    fn a_second_quit_does_not_raise_a_second_prompt() {
        let gate = QuitGate::default();
        assert_eq!(gate.request(true), QuitAction::Prompt);
        assert_eq!(gate.request(true), QuitAction::AlreadyPrompting);
    }

    // The apply may finish while the prompt is up. A second click must still
    // not slip past the outstanding prompt and exit — the user is being asked,
    // and the answer decides.
    #[test]
    fn a_second_quit_does_not_exit_behind_the_prompt() {
        let gate = QuitGate::default();
        assert_eq!(gate.request(true), QuitAction::Prompt);
        assert_eq!(gate.request(false), QuitAction::AlreadyPrompting);
    }

    #[test]
    fn an_unacknowledged_prompt_is_unanswerable() {
        let gate = QuitGate::default();
        let _ = gate.request(true);
        assert!(
            gate.unanswerable(),
            "nothing said the prompt is up, so the quit must not wait on it"
        );
    }

    #[test]
    fn an_acknowledged_prompt_is_answerable() {
        let gate = QuitGate::default();
        let _ = gate.request(true);
        gate.note_shown();
        assert!(
            !gate.unanswerable(),
            "the user is looking at it — waiting is correct"
        );
    }

    #[test]
    fn staying_lets_a_later_quit_prompt_again() {
        let gate = QuitGate::default();
        let _ = gate.request(true);
        gate.note_shown();
        gate.cancel();
        assert_eq!(gate.request(true), QuitAction::Prompt);
        assert!(gate.unanswerable(), "the fresh prompt is not yet up");
    }
}

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
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// What a quit request should do.
#[derive(Debug, PartialEq, Eq)]
pub enum QuitAction {
    /// Nothing is being written — go.
    Exit,
    /// An apply is writing working files: ask before interrupting it. Carries
    /// the request's **generation** — the token its fallback timer and its
    /// acknowledgement must both quote, so neither can speak for a request
    /// that has since been dismissed and replaced.
    Prompt(u64),
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
    /// Bumped on every request and every cancellation. Nothing cancels an
    /// armed fallback timer, so a timer or an acknowledgement from a dismissed
    /// prompt is still in flight; quoting the generation is how it is told
    /// apart from the request now outstanding.
    generation: AtomicU64,
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
        self.shown.store(false, Ordering::SeqCst);
        QuitAction::Prompt(self.generation.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// The window says the prompt is up. Ignored unless it names the request
    /// still outstanding — a late ack from a dismissed prompt would otherwise
    /// suppress the fallback for a window that genuinely cannot ask.
    pub fn note_shown(&self, generation: u64) {
        if self.generation.load(Ordering::SeqCst) == generation {
            self.shown.store(true, Ordering::SeqCst);
        }
    }

    /// The user chose to stay. Bumps the generation, which is what retires the
    /// dismissed prompt's timer and any acknowledgement still on its way.
    pub fn cancel(&self) {
        self.pending.store(false, Ordering::SeqCst);
        self.shown.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The generation now outstanding. For tests and diagnostics — the live
    /// callers always quote the generation their own request handed them.
    #[cfg(test)]
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Is *this* request deferred with **nobody asked** — the prompt never came
    /// up, so no answer is coming and the quit must proceed rather than hang?
    /// Read once after a grace period, never as a running poll, and false for a
    /// request that has since been dismissed.
    #[must_use]
    pub fn unanswerable(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
            && self.pending.load(Ordering::SeqCst)
            && !self.shown.load(Ordering::SeqCst)
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
            !gate.unanswerable(gate.current_generation()),
            "an exit leaves nothing pending to answer"
        );
    }

    #[test]
    fn a_quit_during_an_apply_prompts() {
        let gate = QuitGate::default();
        assert!(matches!(gate.request(true), QuitAction::Prompt(_)));
    }

    #[test]
    fn a_second_quit_does_not_raise_a_second_prompt() {
        let gate = QuitGate::default();
        assert!(matches!(gate.request(true), QuitAction::Prompt(_)));
        assert_eq!(gate.request(true), QuitAction::AlreadyPrompting);
    }

    // The apply may finish while the prompt is up. A second click must still
    // not slip past the outstanding prompt and exit — the user is being asked,
    // and the answer decides.
    #[test]
    fn a_second_quit_does_not_exit_behind_the_prompt() {
        let gate = QuitGate::default();
        assert!(matches!(gate.request(true), QuitAction::Prompt(_)));
        assert_eq!(gate.request(false), QuitAction::AlreadyPrompting);
    }

    #[test]
    fn an_unacknowledged_prompt_is_unanswerable() {
        let gate = QuitGate::default();
        let QuitAction::Prompt(req) = gate.request(true) else {
            panic!("expected a prompt")
        };
        assert!(
            gate.unanswerable(req),
            "nothing said the prompt is up, so the quit must not wait on it"
        );
    }

    #[test]
    fn an_acknowledged_prompt_is_answerable() {
        let gate = QuitGate::default();
        let QuitAction::Prompt(req) = gate.request(true) else {
            panic!("expected a prompt")
        };
        gate.note_shown(req);
        assert!(
            !gate.unanswerable(req),
            "the user is looking at it — waiting is correct"
        );
    }

    #[test]
    fn staying_lets_a_later_quit_prompt_again() {
        let gate = QuitGate::default();
        let QuitAction::Prompt(first) = gate.request(true) else {
            panic!("expected a prompt")
        };
        gate.note_shown(first);
        gate.cancel();
        let QuitAction::Prompt(second) = gate.request(true) else {
            panic!("expected a prompt")
        };
        assert!(gate.unanswerable(second), "the fresh prompt is not yet up");
    }
    // Each prompt arms its own fallback timer, and nothing cancels the old one.
    // Without a generation, a timer armed for a prompt the user dismissed would
    // later read the gate, see the *replacement* request as pending-and-unshown,
    // and exit — cutting short a prompt the user had not answered.
    #[test]
    fn a_stale_request_does_not_speak_for_the_one_that_replaced_it() {
        let gate = QuitGate::default();
        let QuitAction::Prompt(first) = gate.request(true) else {
            panic!("expected a prompt")
        };
        gate.note_shown(first);
        gate.cancel();
        let QuitAction::Prompt(second) = gate.request(true) else {
            panic!("expected a prompt")
        };

        assert_ne!(first, second, "a new request is a new generation");
        assert!(
            !gate.unanswerable(first),
            "the dismissed request's timer must not judge its replacement"
        );
        assert!(
            gate.unanswerable(second),
            "the replacement really is not up yet"
        );
    }

    // The same hazard in the other direction: an acknowledgement for a prompt
    // that has gone away must not mark the new one as on screen, or the
    // fallback would never fire for a window that genuinely cannot ask.
    #[test]
    fn a_stale_acknowledgement_does_not_mark_the_new_prompt_shown() {
        let gate = QuitGate::default();
        let QuitAction::Prompt(first) = gate.request(true) else {
            panic!("expected a prompt")
        };
        gate.cancel();
        let QuitAction::Prompt(second) = gate.request(true) else {
            panic!("expected a prompt")
        };

        gate.note_shown(first);

        assert!(
            gate.unanswerable(second),
            "a late ack from the old prompt must not suppress the new fallback"
        );
    }
}

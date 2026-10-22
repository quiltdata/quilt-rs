//! What a page says about its last copy: the mark on the thing copied, and the
//! words for a screen reader, both for [`CONFIRM_FOR`].
//!
//! Stamped as `components/toasts.rs` is. Every click takes a fresh stamp, and
//! a write settles only while its own stamp is the newest, so two clicks inside
//! one round trip cannot let the slower answer mark something the reader has
//! already moved past, nor an earlier timer clear a later mark.

use std::time::Duration;

use leptos::prelude::*;

/// How long a mark stands, and its words their place in the live region.
///
/// Long enough to be seen after the click that caused it, short enough that a
/// reader returning later is not told something stale about a clipboard
/// anything else may own by then.
pub const CONFIRM_FOR: Duration = Duration::from_millis(1_500);

/// One page's copy state. `T` names what was copied, such as a row's address.
pub struct CopyMark<T: 'static> {
    pub copied: RwSignal<Option<T>>,
    /// For a status region the page draws once, outside anything a re-read or
    /// a re-arranged list replaces: a replaced live region never fires.
    pub said: RwSignal<String>,
    issued: StoredValue<u64>,
}

// By hand: a derive would ask `T` to be `Copy` too.
impl<T> Clone for CopyMark<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CopyMark<T> {}

impl<T: Send + Sync + 'static> CopyMark<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            copied: RwSignal::new(None),
            said: RwSignal::new(String::new()),
            issued: StoredValue::new(0),
        }
    }

    /// Take the next stamp. Everything in flight is stale from here.
    pub fn bump(self) -> u64 {
        self.issued
            .try_update_value(|n| {
                *n += 1;
                *n
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn is_current(self, stamp: u64) -> bool {
        self.issued.try_get_value() == Some(stamp)
    }

    /// Drop the mark and its words, and whatever is still in flight.
    pub fn forget(self) {
        self.bump();
        self.copied.try_set(None);
        self.said.try_set(String::new());
    }

    /// Report one outcome as the mark and the words, and take both back after
    /// [`CONFIRM_FOR`].
    pub fn settle(self, stamp: u64, mark: Option<T>, words: String) {
        if !self.is_current(stamp) {
            return;
        }
        self.copied.try_set(mark);
        self.said.try_set(words);
        set_timeout(
            move || {
                if self.is_current(stamp) {
                    self.copied.try_set(None);
                    self.said.try_set(String::new());
                }
            },
            CONFIRM_FOR,
        );
    }
}

impl<T: PartialEq + Send + Sync + 'static> CopyMark<T> {
    /// Whether `what` is the thing just copied.
    #[must_use]
    pub fn says(self, what: T) -> Signal<bool> {
        Signal::derive(move || self.copied.with(|c| c.as_ref() == Some(&what)))
    }
}

impl<T: Send + Sync + 'static> Default for CopyMark<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// Two clicks inside one round trip: the older write must not land on top
    /// of the newer one, on either surface.
    #[wasm_bindgen_test]
    fn a_slower_copy_cannot_overwrite_a_newer_one() {
        let state = CopyMark::<String>::new();
        let first = state.bump();
        let second = state.bump();

        state.settle(second, Some("two.csv".to_string()), "two".to_string());
        state.settle(first, Some("one.csv".to_string()), "one".to_string());

        assert_eq!(state.said.get_untracked(), "two");
        assert_eq!(state.copied.get_untracked().as_deref(), Some("two.csv"));
    }
}

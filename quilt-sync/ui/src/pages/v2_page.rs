//! The machinery every v2 page that reads data and runs commands shares: one
//! read and its re-reads, one command at a time, and one band for what a
//! command said.
//!
//! # Why a scaffold
//!
//! The package page wrote this inside itself, and the commit page wrote it
//! again and missed parts: the loading frame came back mid-typing, a choice
//! was lost on a re-read, and a popup could write during a publish. Each page
//! that re-derives the rules can miss one, so they live here once.
//!
//! # The rules
//!
//! 1. One read, keyed by the address it was asked for, read again on
//!    [`PageHandle::reload`]: the watcher's news, *Refresh*, a command done.
//! 2. Reads are numbered, and only the newest one's answer lands. An older
//!    read that settles late is dropped.
//! 3. The answer lands in a signal, never a resource. A resource read under
//!    `ByDesign`'s `Suspense` registers with it, and every re-read would put
//!    the loading frame back over the page. And an answer equal to the one on
//!    screen changes nothing, so a re-read that finds nothing new rebuilds
//!    nothing: no focus lost, no row redrawn.
//! 4. What the reader did lives above the rebuild line. The body is rebuilt
//!    from each new answer; a page keeps typed text, a choice or an open dialog
//!    in signals it creates before [`V2Page`], which a rebuild does not
//!    dispose.
//! 5. One lock, [`PageHandle::busy`], held by every command and every popup
//!    that writes.
//! 6. One band under the appbar, [`PageHandle::outcome`], keyed to the
//!    address, so a result that lands after the reader moved on is not drawn.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use crate::components::appbar::appbar_actions;
use crate::kit::readable;
use crate::kit::{Banner, BannerVariant, PageLayout};

/// What a command reported, and which address it reported about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub namespace: String,
    pub variant: BannerVariant,
    /// The page's own sentence. Never the backend's.
    pub lead: String,
    /// The engine's text, when it says something the lead cannot.
    pub detail: Option<String>,
}

/// A v2 page's read: the address in, the answer out.
pub type PageRead<T> = fn(String) -> Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// The page's half of the scaffold: what the body and the commands hold.
///
/// Created by the page before [`V2Page`], so the page can hand it to the
/// state it keeps above the rebuild line.
#[derive(Clone, Copy)]
pub struct PageHandle {
    /// One command at a time. Every control that writes reads this, popups
    /// included, so a second write cannot start on top of the first.
    pub busy: RwSignal<bool>,
    /// What the last command said, and about which address.
    pub outcome: RwSignal<Option<Outcome>>,
    /// Read the page again.
    pub reload: Trigger,
    /// Whether a read is out. It spins *Refresh*, so the button reports a read
    /// the watcher started as readily as one the reader asked for.
    pub in_flight: RwSignal<bool>,
}

impl PageHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            busy: RwSignal::new(false),
            outcome: RwSignal::new(None),
            reload: Trigger::new(),
            in_flight: RwSignal::new(false),
        }
    }

    /// Read `key`'s answer through `read`, now and on every [`Self::reload`]
    /// or change of `key`, and hand back the answer for the address on screen.
    ///
    /// `None` until an answer read for the current `key` lands: an answer for
    /// another address is not yet this one's. A re-read for the same address
    /// keeps the last answer up while it is out. The memo compares answers, so
    /// one equal to the last notifies nothing.
    pub fn read<T>(self, key: Memo<String>, read: PageRead<T>) -> Memo<Option<Result<T, String>>>
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        let Self {
            reload, in_flight, ..
        } = self;
        let landed = RwSignal::new(None::<(String, Result<T, String>)>);
        let reads = StoredValue::new(0_u64);
        Effect::new(move |_| {
            reload.track();
            let asked = key.get();
            reads.update_value(|n| *n += 1);
            let this = reads.get_value();
            in_flight.set(true);
            leptos::task::spawn_local(async move {
                let answer = read(asked.clone()).await;
                // `try_`: the page can be gone by the time the read settles.
                if reads.try_get_value() == Some(this) {
                    in_flight.try_set(false);
                    landed.try_set(Some((asked, answer)));
                }
            });
        });
        Memo::new(move |_| {
            landed.with(|landed| {
                landed
                    .as_ref()
                    .filter(|(read_for, _)| *read_for == key.get())
                    .map(|(_, answer)| answer.clone())
            })
        })
    }
}

impl Default for PageHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `task` holding the page's lock, and retract the band's last outcome as
/// it starts: this command is now the last one, and a failure left up would
/// outlive a retry that succeeds.
pub async fn hold<R>(
    busy: RwSignal<bool>,
    outcome: RwSignal<Option<Outcome>>,
    task: impl Future<Output = R>,
) -> R {
    busy.set(true);
    outcome.set(None);
    let answer = task.await;
    // `try_`: the signal is the page's, and the page can be gone by now.
    busy.try_set(false);
    answer
}

/// A v2 page's frame: the appbar with *Refresh*, the outcome band and the
/// page's own bands under it, and the body drawn from the answer.
///
/// `body` runs once per new answer. Anything that must outlive one belongs to
/// the page, created before this component; see the module's rule 4.
#[component]
pub fn V2Page<T, B, D, F>(
    /// The page's name, for a screen reader. Not reactive.
    heading: &'static str,
    page: PageHandle,
    /// The answer, from [`PageHandle::read`].
    answer: Memo<Option<Result<T, String>>>,
    /// The address on screen, which keys the outcome band.
    #[prop(into)]
    showing: Signal<String>,
    /// The body before the first answer: the page's first paint.
    skeleton: fn() -> AnyView,
    /// The body when the read failed.
    failure: F,
    /// The answered body.
    body: D,
    /// Bands the answer calls for, under the outcome band.
    bands: B,
) -> impl IntoView
where
    T: Clone + PartialEq + Send + Sync + 'static,
    B: Fn(T) -> AnyView + Send + Sync + 'static,
    D: Fn(T) -> AnyView + Send + Sync + 'static,
    F: Fn() -> AnyView + Send + Sync + 'static,
{
    let PageHandle {
        outcome,
        reload,
        in_flight,
        ..
    } = page;
    view! {
        <PageLayout
            heading=heading
            // In the frame's slot, directly under the appbar, pushing the page
            // down. Nothing before the first answer: a skeleton here would
            // reserve a band for news that usually is not there.
            banner=view! {
                {outcome_band(outcome, showing)}
                // A failed read says nothing about a command that ran before
                // it, which is why the outcome band is outside this.
                {move || match answer.get() {
                    Some(Ok(d)) => bands(d),
                    Some(Err(_)) | None => ().into_any(),
                }}
            }
                .into_any()
            actions=appbar_actions(move || reload.notify(), in_flight.into())
        >
            {move || match answer.get() {
                None => skeleton(),
                Some(Ok(d)) => body(d),
                Some(Err(_)) => failure(),
            }}
        </PageLayout>
    }
}

/// What the last command said, while it is about the address on screen.
///
/// One route serves every package, so a command's result can arrive after the
/// reader has moved to another one. It is dropped rather than drawn: a reader
/// cannot tell a stale outcome from a fresh one by its text.
///
/// The lead is the page's and the detail is the engine's: the vocabulary is
/// UI-owned, and the engine's refusal is the part nothing else knows.
pub(super) fn outcome_band(outcome: RwSignal<Option<Outcome>>, showing: Signal<String>) -> AnyView {
    let mine = move || outcome.get().filter(|o| o.namespace == showing.get());
    view! {
        <Show when=move || mine().is_some() fallback=|| ()>
            {
                let said = mine().expect("checked by the guard above");
                view! {
                    <Banner
                        variant=said.variant
                        on_dismiss=move |_| outcome.set(None)
                    >
                        {said.lead}
                        {said.detail.map(|detail| view! { " " {readable(&detail)} })}
                    </Banner>
                }
            }
        </Show>
    }
    .into_any()
}

/// A failure the page words, with the engine's text after it.
#[must_use]
pub fn critical(namespace: String, lead: &str, detail: Option<String>) -> Outcome {
    Outcome {
        namespace,
        variant: BannerVariant::Critical,
        lead: lead.to_string(),
        detail,
    }
}

/// The scaffold's life, over a small page mounted as the routes mount theirs:
/// under `ByDesign`'s `Suspense`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, mount, sleep_ms, unmount_earlier};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    thread_local! {
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        /// What each read answers, by read number from 1; the last one repeats.
        static ANSWERS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
        /// How long each read takes, by read number from 1; 0 past the end.
        static DELAYS: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
        static HANDLE: std::cell::Cell<Option<PageHandle>> = const { std::cell::Cell::new(None) };
    }

    fn scripted(_: String) -> Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        let n = READS.with(|r| {
            r.set(r.get() + 1);
            r.get()
        }) as usize;
        let answer = ANSWERS.with(|a| {
            let a = a.borrow();
            a[(n - 1).min(a.len() - 1)]
        });
        let delay = DELAYS.with(|d| d.borrow().get(n - 1).copied().unwrap_or(0));
        Box::pin(async move {
            if delay > 0 {
                sleep_ms(delay).await;
            }
            Ok(answer)
        })
    }

    fn pending(_: String) -> Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        READS.with(|r| r.set(r.get() + 1));
        Box::pin(std::future::pending())
    }

    /// A page whose body draws the answer and a field. The field's text is
    /// the page's, above the rebuild line.
    #[component]
    fn Probe(read: PageRead<u32>) -> impl IntoView {
        let page = PageHandle::new();
        HANDLE.with(|h| h.set(Some(page)));
        let key = Memo::new(|_| "team/a".to_string());
        let answer = page.read(key, read);
        let typed = RwSignal::new(String::new());
        view! {
            <V2Page
                heading="Probe"
                page=page
                answer=answer
                showing=key
                skeleton=|| view! { <p data-skeleton>"…"</p> }.into_any()
                bands=|_| ().into_any()
                failure=|| view! { <p>"failed"</p> }.into_any()
                body=move |n: u32| {
                    view! {
                        <p data-answer>{n}</p>
                        <input
                            data-field
                            prop:value=move || typed.get_untracked()
                            on:input=move |ev| typed.set(event_target_value(&ev))
                        />
                    }
                    .into_any()
                }
            />
        }
    }

    async fn probe(answers: Vec<u32>, delays: Vec<i32>, read: PageRead<u32>) -> web_sys::Element {
        unmount_earlier();
        READS.with(|r| r.set(0));
        ANSWERS.with(|a| *a.borrow_mut() = answers);
        DELAYS.with(|d| *d.borrow_mut() = delays);
        let el = mount(move || {
            view! {
                <leptos_router::components::Router>
                    <Suspense fallback=|| view! { <p data-fallback>"loading"</p> }>
                        <Probe read=read />
                    </Suspense>
                </leptos_router::components::Router>
            }
        });
        sleep_ms(30).await;
        el
    }

    fn handle() -> PageHandle {
        HANDLE
            .with(std::cell::Cell::get)
            .expect("the probe mounted")
    }

    fn shown(el: &web_sys::Element) -> Option<String> {
        el.query_selector("[data-answer]")
            .unwrap()
            .map(|p| p.text_content().unwrap_or_default())
    }

    fn field(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("[data-field]")
            .unwrap()
            .unwrap_or_else(|| panic!("the field; markup was {}", el.inner_html()))
            .unchecked_into()
    }

    fn type_into(input: &web_sys::HtmlInputElement, text: &str) {
        input.set_value(text);
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    fn focused() -> Option<web_sys::Element> {
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
    }

    /// A re-read that finds what is on screen rebuilds nothing: the field the
    /// reader is typing in stays, and so does the cursor in it.
    #[wasm_bindgen_test]
    async fn an_unchanged_re_read_keeps_the_field_and_the_focus() {
        let el = probe(vec![1], vec![], scripted).await;
        let input = field(&el);
        input.focus().unwrap();
        type_into(&input, "Plate 7");

        handle().reload.notify();
        sleep_ms(30).await;

        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert!(input.is_connected(), "the field was rebuilt");
        assert_eq!(
            focused(),
            Some(input.clone().unchecked_into()),
            "the cursor stayed"
        );
        assert_eq!(input.value(), "Plate 7");
    }

    /// A re-read that finds news rebuilds the body, and what the reader typed
    /// survives it, because it is the page's.
    #[wasm_bindgen_test]
    async fn a_changed_re_read_keeps_what_was_typed() {
        let el = probe(vec![1, 2], vec![], scripted).await;
        type_into(&field(&el), "Plate 7");

        handle().reload.notify();
        sleep_ms(30).await;

        assert_eq!(shown(&el).as_deref(), Some("2"));
        assert_eq!(field(&el).value(), "Plate 7");
    }

    /// While a re-read is out, the last answer stays up: no loading frame from
    /// the `Suspense` above, no skeleton, and *Refresh* spins.
    #[wasm_bindgen_test]
    async fn a_re_read_in_flight_keeps_the_page() {
        let el = probe(vec![1, 2], vec![0, 10_000], scripted).await;

        button_saying(&el, "Refresh").click();
        sleep_ms(30).await;

        assert!(handle().in_flight.get_untracked(), "the read is out");
        assert!(
            el.query_selector("[data-fallback], [data-skeleton]")
                .unwrap()
                .is_none(),
            "the page went back to a loading frame; markup was {}",
            el.inner_html()
        );
        assert_eq!(shown(&el).as_deref(), Some("1"));
    }

    /// Two reads overlap and the older settles last: only the newer lands.
    #[wasm_bindgen_test]
    async fn only_the_newest_of_two_reads_lands() {
        // The first read answers at once, the second late, the third early.
        let el = probe(vec![1, 2, 3], vec![0, 150, 10], scripted).await;
        handle().reload.notify();
        sleep_ms(5).await;
        handle().reload.notify();
        sleep_ms(250).await;

        assert_eq!(READS.with(std::cell::Cell::get), 3);
        assert_eq!(
            shown(&el).as_deref(),
            Some("3"),
            "the older read landed last"
        );
        assert!(!handle().in_flight.get_untracked());
    }

    /// The first paint is the skeleton, inside the frame: the `Suspense` above
    /// never shows its own.
    #[wasm_bindgen_test]
    async fn the_first_read_draws_the_skeleton_not_the_loading_frame() {
        let el = probe(vec![1], vec![], pending).await;
        assert!(el.query_selector("[data-skeleton]").unwrap().is_some());
        assert!(el.query_selector("[data-fallback]").unwrap().is_none());
    }

    /// A command holds the lock every writer reads, popups included, and
    /// retracts the last outcome as it starts.
    #[wasm_bindgen_test]
    async fn a_command_holds_the_lock_and_retracts_the_last_outcome() {
        let _el = probe(vec![1], vec![], scripted).await;
        let page = handle();
        page.outcome
            .set(Some(critical("team/a".into(), "Could not save.", None)));

        leptos::task::spawn_local(hold(page.busy, page.outcome, sleep_ms(50)));
        sleep_ms(10).await;
        assert!(page.busy.get_untracked(), "held while it runs");
        assert_eq!(page.outcome.get_untracked(), None, "retracted");

        sleep_ms(80).await;
        assert!(!page.busy.get_untracked(), "released when it settles");
    }

    /// The band draws an outcome for the address on screen only.
    #[wasm_bindgen_test]
    async fn the_band_is_keyed_to_the_address() {
        let el = probe(vec![1], vec![], scripted).await;
        let page = handle();
        let alert = || el.query_selector("[role=alert]").unwrap().is_some();

        page.outcome
            .set(Some(critical("team/b".into(), "Could not save.", None)));
        sleep_ms(10).await;
        assert!(!alert(), "another package's news is not drawn");

        page.outcome
            .set(Some(critical("team/a".into(), "Could not save.", None)));
        sleep_ms(10).await;
        assert!(alert(), "this package's is; markup was {}", el.inner_html());
    }
}

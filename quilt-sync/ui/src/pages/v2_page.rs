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
//!    [`PageHandle::reload`]: the watcher's news, *Refresh*, a command done, a
//!    reconnect to the engine after news may have been lost.
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
//! 7. A read that fails for a missing session or setup goes to `/login` or
//!    `/setup`, as v1 does, but only before the page has drawn an answer for
//!    its address. After that the page may hold the reader's work, so a failed
//!    re-read stays and says so.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use crate::components::appbar::{appbar_actions, transit_appbar_actions};
use crate::error_handler::{current_path, follow_redirect};
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
    /// How many reads have landed, failed ones included. A command that asks
    /// for a re-read waits for this to pass the count it saw.
    pub landed: RwSignal<u64>,
}

impl PageHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            busy: RwSignal::new(false),
            outcome: RwSignal::new(None),
            reload: Trigger::new(),
            in_flight: RwSignal::new(false),
            landed: RwSignal::new(0),
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
        crate::engine::reread_on_reconnect(self.reload);
        self.answer(key, read)
    }

    /// [`Self::read`] for a page whose read is its operation, as a relay's: run
    /// once per address, and never again on its own after a reconnect, since
    /// the first run may still be going on the engine.
    pub fn run<T>(self, key: Memo<String>, op: PageRead<T>) -> Memo<Option<Result<T, String>>>
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        self.answer(key, op)
    }

    fn answer<T>(self, key: Memo<String>, read: PageRead<T>) -> Memo<Option<Result<T, String>>>
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        let Self {
            reload,
            in_flight,
            landed: count,
            ..
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
                    count.try_update(|n| *n += 1);
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
    /// The appbar's controls; *Refresh* and Settings by default. Settings
    /// itself has none.
    #[prop(optional)]
    actions: Option<AnyView>,
    /// The page's mount is its operation, so a reload or a Back onto it would
    /// run it again: *Refresh* is drawn disabled and every way out of the
    /// appbar replaces this history entry. Wins over `actions`.
    #[prop(optional)]
    transit: bool,
    /// The page scrolls as one, under the parts it holds: `PageLayout`'s
    /// `page_scroll`.
    #[prop(optional)]
    page_scroll: bool,
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
    let on = current_path();
    // The address whose answer the body last drew.
    let drawn_for = StoredValue::new(None::<String>);
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
            actions=if transit {
                transit_appbar_actions()
            } else {
                actions.unwrap_or_else(|| appbar_actions(move || reload.notify(), in_flight.into()))
            }
            replace=transit
            page_scroll=page_scroll
        >
            {move || match answer.get() {
                None => skeleton(),
                Some(Ok(d)) => {
                    drawn_for.set_value(Some(showing.get_untracked()));
                    body(d)
                }
                // A missing session or setup goes where v1 sends it, but only
                // from a page with nothing on screen for this address: one
                // that has drawn may hold the reader's work.
                Some(Err(e))
                    if drawn_for.with_value(|k| k.as_deref() != Some(&*showing.get_untracked()))
                        && follow_redirect(&e, &on) =>
                {
                    skeleton()
                }
                Some(Err(_)) => failure(),
            }}
        </PageLayout>
    }
}

/// What the last command said, while it is about the address on screen.
///
/// One route serves every package, so a command's result can arrive after the
/// reader has moved to another one. It is dropped rather than drawn: a reader
/// cannot tell a stale outcome from a fresh one by its text. Dropped, not just
/// hidden: kept, it would come back as fresh news when the reader returns to
/// the package it was about.
///
/// The lead is the page's and the detail is the engine's: the vocabulary is
/// UI-owned, and the engine's refusal is the part nothing else knows.
pub(super) fn outcome_band(outcome: RwSignal<Option<Outcome>>, showing: Signal<String>) -> AnyView {
    // `try_`: the outcome is the page's and can be gone before this first runs.
    Effect::new(move |_| {
        let Some(here) = showing.try_get() else {
            return;
        };
        if outcome
            .try_with(|o| o.as_ref().is_some_and(|o| o.namespace != here))
            .unwrap_or(false)
        {
            outcome.try_set(None);
        }
    });
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

/// Whether the page that started a command on address `here` is still up and
/// still on it, for a command whose answer moves the reader on: a late answer
/// must not take them from wherever they went meanwhile.
#[must_use]
pub fn still_here(key: Memo<String>, here: &str) -> bool {
    // `try_`: the page can be gone by the time the answer lands.
    key.try_get_untracked().as_deref() == Some(here)
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
        static CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn counted(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        CALLS.with(|c| c.set(c.get() + 1));
        Box::pin(async { Ok(1) })
    }

    /// After a reconnect a page reads again; a relay, whose read is its
    /// operation, does not run again on its own.
    #[wasm_bindgen_test]
    async fn a_reconnect_reads_a_page_again_but_never_reruns_a_relay() {
        for (relay, expected) in [(false, 2), (true, 1)] {
            CALLS.with(|c| c.set(0));
            let engine = crate::engine::fake::FakeEngine::install();
            mount(move || {
                let key = Memo::new(|_| "k".to_string());
                let page = PageHandle::new();
                let _answer = if relay {
                    page.run(key, counted)
                } else {
                    page.read(key, counted)
                };
            });
            let _listener = crate::engine::listen::<serde_json::Value>("toast", |_| ());
            sleep_ms(0).await;
            engine.hello();
            engine.close();
            sleep_ms(0).await;
            engine.hello();
            sleep_ms(0).await;
            assert_eq!(CALLS.with(std::cell::Cell::get), expected, "relay: {relay}");
            unmount_earlier();
        }
    }

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

    fn setup_required(_: String) -> Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        Box::pin(async { Err(r#"{"kind":"setup_required","message":"Set up"}"#.to_string()) })
    }

    fn session_absent(_: String) -> Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        Box::pin(async {
            Err(r#"{"kind":"session_absent","message":"Log in","host":"quilt.test"}"#.to_string())
        })
    }

    /// The probe at `at`, over `read`: where the window is afterwards, and
    /// whether the page drew its failure. Puts the window back after.
    async fn probe_at(at: &str, read: PageRead<u32>) -> (String, String, bool) {
        let window = web_sys::window().unwrap();
        let history = window.history().unwrap();
        let was = window.location().href().unwrap();
        history
            .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(at))
            .unwrap();
        let before = history.length().unwrap();
        let el = probe(vec![1], vec![], read).await;
        let location = window.location();
        let landed = format!(
            "{}{}",
            location.pathname().unwrap(),
            location.search().unwrap()
        );
        assert_eq!(history.length().unwrap(), before, "the entry is replaced");
        let failed = el.text_content().unwrap_or_default().contains("failed");
        unmount_earlier();
        history
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&was))
            .unwrap();
        (landed, at.to_string(), failed)
    }

    /// A read that needs setup goes to `/setup`, as v1 does, and draws no
    /// failure on the way.
    #[wasm_bindgen_test]
    async fn a_read_that_needs_setup_goes_to_setup() {
        let (landed, _, failed) =
            probe_at("/installed-package?namespace=team%2Fa", setup_required).await;
        assert_eq!(landed, "/setup");
        assert!(!failed);
    }

    /// A read with no session goes to `/login` for its host, coming back here.
    #[wasm_bindgen_test]
    async fn a_read_with_no_session_goes_to_login_and_back() {
        let (landed, at, failed) =
            probe_at("/installed-package?namespace=team%2Fa", session_absent).await;
        let back = urlencoding::encode(&at).into_owned();
        assert_eq!(landed, format!("/login?host=quilt.test&back={back}"));
        assert!(!failed);
    }

    /// Answers 1, then a missing session.
    fn lapses(_: String) -> Pin<Box<dyn Future<Output = Result<u32, String>>>> {
        let n = READS.with(|r| {
            r.set(r.get() + 1);
            r.get()
        });
        Box::pin(async move {
            if n == 1 {
                Ok(1)
            } else {
                Err(
                    r#"{"kind":"session_absent","message":"Log in","host":"quilt.test"}"#
                        .to_string(),
                )
            }
        })
    }

    /// A page that has drawn its address stays when a re-read finds the
    /// session gone, and keeps what was typed for when it reads again.
    #[wasm_bindgen_test]
    async fn a_failed_re_read_after_an_answer_stays() {
        let window = web_sys::window().unwrap();
        let was = window.location().href().unwrap();
        let history = window.history().unwrap();
        history
            .push_state_with_url(
                &wasm_bindgen::JsValue::NULL,
                "",
                Some("/commit?namespace=team%2Fa"),
            )
            .unwrap();
        let el = probe(vec![1], vec![], lapses).await;
        type_into(&field(&el), "Plate 7");
        handle().reload.notify();
        sleep_ms(30).await;

        let landed = window.location().pathname().unwrap();
        let failed = el.text_content().unwrap_or_default().contains("failed");
        history
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&was))
            .unwrap();
        assert_eq!(landed, "/commit", "the page stayed");
        assert!(
            failed,
            "and said the read failed; markup was {}",
            el.inner_html()
        );
    }

    /// `/setup` and `/login` reading the error they exist for stay put and say
    /// so, rather than sending the reader to themselves.
    #[wasm_bindgen_test]
    async fn setup_and_login_never_redirect_to_themselves() {
        let (landed, _, failed) = probe_at("/setup", setup_required).await;
        assert_eq!(landed, "/setup");
        assert!(failed, "setup draws its own failure");
        let (landed, _, failed) = probe_at("/login?host=quilt.test&back=%2F", session_absent).await;
        assert_eq!(landed, "/login?host=quilt.test&back=%2F");
        assert!(failed, "login draws its own failure");
    }

    /// A command's answer moves the reader on only from the page and address
    /// it started on.
    #[wasm_bindgen_test]
    fn a_late_answer_is_still_here_only_on_its_own_page() {
        let owner = Owner::new();
        let (key, address) = owner.with(|| {
            let address = RwSignal::new("team/a".to_string());
            (Memo::new(move |_| address.get()), address)
        });
        assert!(still_here(key, "team/a"));
        assert!(!still_here(key, "team/b"), "another address");
        address.set("team/b".to_string());
        assert!(!still_here(key, "team/a"), "the address changed");
        owner.cleanup();
        assert!(!still_here(key, "team/b"), "the page is gone");
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
        assert_eq!(
            page.outcome.get_untracked(),
            None,
            "and is dropped, so it cannot come back as fresh news"
        );

        page.outcome
            .set(Some(critical("team/a".into(), "Could not save.", None)));
        sleep_ms(10).await;
        assert!(alert(), "this package's is; markup was {}", el.inner_html());
    }
}

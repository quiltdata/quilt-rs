//! A port of the catalog's `JsonDisplay`: a JSON tree where each array and
//! object is one line, fitted by [`json_oneliner::print`], until opened.
//!
//! Differences from the catalog: a branch opens on a native `<button>`; two
//! text levels instead of opacity steps; URLs open only through
//! `on_open_url`; folded strings are drawn escaped so the line stays one line;
//! closed branches draw nothing; the budget is measured in the face's `ch`.

use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::icons;
use super::json_oneliner::{self, Kind, Part};

stylance::import_crate_style!(style, "src/kit/json_display.module.scss");

/// In `ch`; the stylesheet uses the same numbers.
const ICON_CH: f64 = 2.0;
const INDENT_CH: f64 = 2.0;
/// The catalog's reserve: a separator, a `<…N>`, and the braces' spaces.
const RESERVED_CH: f64 = 2.0 + 4.0 + 4.0;

/// How many levels start open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expanded {
    #[default]
    None,
    Levels(usize),
    All,
}

impl Expanded {
    fn open(self) -> bool {
        match self {
            Self::None | Self::Levels(0) => false,
            Self::Levels(_) | Self::All => true,
        }
    }

    fn deeper(self) -> Self {
        match self {
            Self::Levels(n) => Self::Levels(n.saturating_sub(1)),
            other => other,
        }
    }
}

#[derive(Clone, Copy)]
struct Ctx {
    /// The top line's room, in characters.
    budget: Signal<f64>,
    show_values: bool,
    on_open_url: Option<Callback<String>>,
}

#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn JsonDisplay(
    value: Value,
    #[prop(optional, into)] name: Option<String>,
    #[prop(optional)] expanded: Expanded,
    /// Values on a folded line, not just keys. Arrays always show theirs.
    #[prop(optional, default = true)]
    show_values: bool,
    /// A fixed room in characters instead of the measured width.
    #[prop(optional)]
    chars: Option<f64>,
    #[prop(optional)] on_open_url: Option<Callback<String>>,
) -> impl IntoView {
    let root: NodeRef<leptos::html::Div> = NodeRef::new();
    let probe: NodeRef<leptos::html::Span> = NodeRef::new();
    let measured = RwSignal::new(0.0_f64);
    if chars.is_none() {
        measure(root, probe, measured);
    }
    let budget = Signal::derive(move || chars.unwrap_or_else(|| measured.get()));
    let ctx = Ctx {
        budget,
        show_values,
        on_open_url,
    };

    view! {
        <div class=style::root node_ref=root>
            // Measured to turn the width into characters.
            <span class=style::probe node_ref=probe aria-hidden="true">
                "0000000000"
            </span>
            {entry(ctx, name.as_deref(), &value, true, expanded, 0.0)}
        </div>
    }
}

type Observing = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// Keep `out` at the number of characters the root's width holds.
fn measure(
    root: NodeRef<leptos::html::Div>,
    probe: NodeRef<leptos::html::Span>,
    out: RwSignal<f64>,
) {
    let update = move || {
        let (Some(root), Some(probe)) = (root.get_untracked(), probe.get_untracked()) else {
            return;
        };
        let ch = probe.get_bounding_client_rect().width() / 10.0;
        if ch > 0.0 {
            out.set(f64::from(root.client_width()) / ch);
        }
    };
    let held: StoredValue<Option<Observing>, LocalStorage> = StoredValue::new_local(None);
    root.on_load(move |el| {
        update();
        let callback = Closure::<dyn FnMut()>::new(update);
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&el);
            held.set_value(Some((observer, callback)));
        }
    });
    on_cleanup(move || {
        held.try_update_value(|held| {
            if let Some((observer, _)) = held.take() {
                observer.disconnect();
            }
        });
    });
}

/// `indent` is in characters from the root.
fn entry(
    ctx: Ctx,
    name: Option<&str>,
    value: &Value,
    top: bool,
    expanded: Expanded,
    indent: f64,
) -> AnyView {
    match value {
        Value::Array(_) | Value::Object(_) => branch(ctx, name, value.clone(), expanded, indent),
        _ => leaf(ctx, name, value, top),
    }
}

fn key(name: Option<&str>) -> Option<AnyView> {
    name.map(|name| {
        view! {
            <span class=style::key>{name.to_string()}</span>
            <span class=style::muted>": "</span>
        }
        .into_any()
    })
}

fn leaf(ctx: Ctx, name: Option<&str>, value: &Value, top: bool) -> AnyView {
    view! {
        <div class=style::line>
            {(!top).then(|| view! { <span class=style::icon /> })}
            {key(name)}
            {scalar(ctx, value)}
        </div>
    }
    .into_any()
}

fn scalar(ctx: Ctx, value: &Value) -> AnyView {
    let Value::String(s) = value else {
        return view! { <span class=style::value>{value.to_string()}</span> }.into_any();
    };
    let url = (s.starts_with("https://") || s.starts_with("http://"))
        .then_some(ctx.on_open_url)
        .flatten();
    let text = match url {
        Some(open) => {
            let href = s.clone();
            view! {
                <button
                    type="button"
                    class=style::link
                    on:click=move |_| open.run(href.clone())
                >
                    {s.clone()}
                </button>
            }
            .into_any()
        }
        None => view! { <span class=style::value>{s.clone()}</span> }.into_any(),
    };
    view! {
        <span class=style::muted>"\""</span>
        {text}
        <span class=style::muted>"\""</span>
    }
    .into_any()
}

fn branch(ctx: Ctx, name: Option<&str>, value: Value, expanded: Expanded, indent: f64) -> AnyView {
    let array = value.is_array();
    // Arrays are keyed by index once open, as in the catalog.
    let children: Vec<(Option<String>, Value)> = match &value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (Some(i.to_string()), v.clone()))
            .collect(),
        Value::Object(fields) => fields
            .iter()
            .map(|(k, v)| (Some(k.clone()), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let (open_brace, close_brace) = if array { ("[", "]") } else { ("{", "}") };
    let empty = children.is_empty();
    let open = RwSignal::new(!empty && expanded.open());

    #[allow(clippy::cast_precision_loss, reason = "a key's length in characters")]
    let key_ch = name.map_or(0.0, |n| n.chars().count() as f64 + 2.0);
    let budget = ctx.budget;
    let show_values = ctx.show_values || array;
    let folded = move || {
        let room = budget.get() - indent - ICON_CH - key_ch - RESERVED_CH;
        let line = json_oneliner::print(&value, room, show_values);
        folded_line(&line.parts)
    };

    let head = view! {
        <span class=style::icon aria-hidden="true">
            {move || {
                (!empty).then(|| if open.get() { icons::chevron_down() } else { icons::chevron_right() })
            }}
        </span>
        {key(name)}
        {move || {
            if open.get() {
                view! { <span class=style::muted>{open_brace}</span> }.into_any()
            } else {
                folded().into_any()
            }
        }}
    };

    let line = if empty {
        view! { <div class=style::line>{head}</div> }.into_any()
    } else {
        view! {
            <button
                type="button"
                class=style::toggle
                aria-expanded=move || if open.get() { "true" } else { "false" }
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {head}
            </button>
        }
        .into_any()
    };

    let inner_indent = indent + INDENT_CH;
    view! {
        <div>
            {line}
            {move || {
                open.get()
                    .then(|| {
                        let rows = children
                            .iter()
                            .map(|(k, v)| {
                                entry(ctx, k.as_deref(), v, false, expanded.deeper(), inner_indent)
                            })
                            .collect_view();
                        view! {
                            <div class=style::inner>
                                {rows}
                                <div class=style::muted>{close_brace}</div>
                            </div>
                        }
                    })
            }}
        </div>
    }
    .into_any()
}

/// A folded line's parts. `[  ]` collapses to `[]`, as in the catalog.
fn folded_line(parts: &[Part]) -> impl IntoView + use<> {
    let brace_trimmed = |i: usize, part: &Part| -> String {
        let prev = i.checked_sub(1).and_then(|j| parts.get(j));
        let next = parts.get(i + 1);
        let meets = |other: Option<&Part>, other_end: bool| {
            other.is_some_and(|o| {
                o.kind == Kind::Brace
                    && if other_end {
                        o.value.ends_with(' ') && part.value.starts_with(' ')
                    } else {
                        o.value.starts_with(' ') && part.value.ends_with(' ')
                    }
            })
        };
        if meets(prev, true) || meets(next, false) {
            part.value.trim().to_string()
        } else {
            part.value.clone()
        }
    };
    let spans = parts
        .iter()
        .enumerate()
        .map(|(i, part)| match part.kind {
            Kind::Key => view! { <span class=style::key>{part.value.clone()}</span> }.into_any(),
            Kind::Brace => {
                view! { <span class=style::muted>{brace_trimmed(i, part)}</span> }.into_any()
            }
            Kind::Separator | Kind::Equal | Kind::More => {
                view! { <span class=style::muted>{part.value.clone()}</span> }.into_any()
            }
            Kind::String => view! {
                <span class=style::muted>"\""</span>
                <span class=style::value>{part.text.clone().unwrap_or_default()}</span>
                <span class=style::muted>"\""</span>
            }
            .into_any(),
            Kind::Primitive | Kind::Object => {
                view! { <span class=style::value>{part.value.clone()}</span> }.into_any()
            }
        })
        .collect_view();
    view! { <span class=style::folded>{spans}</span> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms};
    use serde_json::json;
    use wasm_bindgen_test::*;

    fn text(el: &web_sys::Element) -> String {
        el.text_content().unwrap_or_default()
    }

    /// At 62 wide the printer has 50, a catalog test's budget.
    #[wasm_bindgen_test]
    fn a_folded_line_reads_as_the_catalog_prints_it() {
        let el = mount(|| {
            view! {
                <JsonDisplay
                    value=json!({
                        "A": [1, 2, 3],
                        "B": "Lorem",
                        "C": {"a": 1, "b": 2},
                        "D": {"a": [1, {"c": 3}, "a"], "b": 2},
                        "E": [1, {"b": ["c", {"d": "e"}]}, "a"],
                    })
                    chars=62.0
                />
            }
        });
        let button = el.query_selector("button").unwrap().expect("a toggle");
        assert_eq!(
            text(&button),
            r#"{ A: [ 1, <…2> ], B: "Lorem", C: { <…2> }, <…2> }"#
        );
        assert_eq!(
            button.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
    }

    #[wasm_bindgen_test]
    async fn the_button_opens_the_branch() {
        let el = mount(|| view! { <JsonDisplay value=json!({"plate": 7}) chars=80.0 /> });
        assert!(!text(&el).contains("plate: 7\n"), "{}", text(&el));
        let button: web_sys::HtmlElement = el
            .query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        button.click();
        sleep_ms(0).await;
        assert_eq!(
            button.get_attribute("aria-expanded").as_deref(),
            Some("true")
        );
        let lines = el
            .query_selector_all("[class*='inner'] > *")
            .unwrap()
            .length();
        assert_eq!(
            lines,
            2,
            "one field and the closing brace: {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn an_empty_object_is_not_a_control() {
        let el = mount(|| view! { <JsonDisplay value=json!({}) chars=80.0 /> });
        assert!(el.query_selector("button").unwrap().is_none());
        assert!(text(&el).ends_with("{}"), "{}", text(&el));
    }
}

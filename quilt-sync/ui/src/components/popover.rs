use leptos::prelude::*;

/// Wrap `content` in the app's hover popover when there is something to say,
/// and return it untouched when there is not.
///
/// The markup is the established `.qui-popover` / `.popover-wrapper` /
/// `.popover` trio (see `assets/css/components/popover.css`), which reveals the
/// bubble on hover of the *wrapper* — so it also explains a **disabled**
/// control, which carries no hover state of its own.
///
/// Returning the content bare on `None` keeps the surrounding layout identical
/// in the common case: no extra element joins the flex row unless there is a
/// reason to show one.
///
/// A plain function rather than a `#[component]` because Leptos's `Children`
/// is `Send`, and the action-bar buttons this wraps close over `Rc` handlers.
pub fn with_popover(text: Option<String>, content: AnyView) -> AnyView {
    let Some(text) = text else {
        return content;
    };
    view! {
        <div class="qui-popover">
            {content}
            <div class="popover-wrapper">
                <div class="popover">{text}</div>
            </div>
        </div>
    }
    .into_any()
}

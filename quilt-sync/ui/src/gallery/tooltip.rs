//! `Tooltip` stories.
//!
//! Nothing here forces one open. A tooltip shows on hover or on keyboard focus,
//! and a gallery-only prop that pinned it open would be a mode the app never
//! has — the cell would show a surface no reader can produce, and hide the
//! timing, which is half of what is under review. So every cell is a trigger,
//! and the note says how to ask.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ActionMenu;
use crate::kit::Align;
use crate::kit::Button;
use crate::kit::MenuAction;
use crate::kit::Tooltip;

/// A button that names its tooltip, which is the wiring every trigger owes:
/// the tooltip cannot reach into what the closure draws.
///
/// A text button, not an `IconButton`: an icon button's name is also its
/// `title`, and an element with both shows the browser's box over the kit's a
/// second later. A trigger carries no `title`.
fn button_trigger(label: &'static str) -> impl FnOnce(String) -> AnyView {
    move |id| view! { <Button on_click=|_| () aria_describedby=id>{label}</Button> }.into_any()
}

fn hint(words: &'static str) -> Signal<String> {
    Signal::stored(words.to_string())
}

const NOTE: &str = "What shows on hover or focus, in place of a `title`: plain text in the \
                    platform's own popover, explaining a mark and never being one. Nothing \
                    here is open — rest the pointer on a trigger for half a second, or Tab to \
                    one; a click's focus does not open it. Move onto the words and they stay; \
                    move away and they go a moment later. Escape, a click, a scroll or a \
                    resize closes it.";

#[component]
pub fn TooltipStories() -> impl IntoView {
    let menu = vec![
        MenuAction::new("Open file", Callback::new(|()| ())),
        MenuAction::new("Copy URI", Callback::new(|()| ())),
    ];

    view! {
        <Story title="Tooltip" note=NOTE>
            <Cell label="on an icon button — hover it, or Tab to it">
                <Tooltip
                    text=hint("Checks the platform for revisions published since this page loaded.")
                    trigger=button_trigger("Refresh")
                />
            </Cell>
            <Cell label="beside an open menu — open the dots, then hover the other button; the menu stays">
                <div class="g-inline">
                    <ActionMenu aria_label="More actions for this file" actions=menu />
                    <Tooltip
                        text=hint("Opening this sentence closes no menu: it is a manual popover.")
                        trigger=button_trigger("Refresh")
                    />
                </div>
            </Cell>
            <Cell full=true label="at the right edge — hangs leftwards from its trigger (Align::End)">
                <div class="g-inline g-inline--end" style="width:100%">
                    <Tooltip
                        text=hint("A trailing trigger lines up its right edge, so the words stay beside it at any window width.")
                        align=Align::End
                        trigger=button_trigger("More")
                    />
                </div>
            </Cell>
            <Cell full=true label="near the bottom — scroll until this row touches the window's bottom edge, then hover: it opens above">
                <Tooltip
                    text=hint("Below by preference, above when below does not fit.")
                    trigger=button_trigger("Settings")
                />
            </Cell>
            <Cell full=true label="inside a scrolling list — hover a mark, then scroll the list: it closes">
                <div style="width:100%; max-height:120px; overflow-y:auto; \
                            border:1px solid var(--q-borderColor-muted); \
                            border-radius:var(--q-radius)">
                    {(1..=8)
                        .map(|n| {
                            view! {
                                <div style="display:flex; align-items:center; \
                                            gap:var(--q-space-2); \
                                            padding:var(--q-space-1) var(--q-space-3); \
                                            border-bottom:1px solid var(--q-borderColor-muted)">
                                    <Tooltip
                                        text=hint("This sentence is anchored to its row; a scroll moves the row, so the sentence goes.")
                                        trigger=button_trigger("About")
                                    />
                                    <span>{format!("plate-{n:02}.csv")}</span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </Cell>
        </Story>
    }
}

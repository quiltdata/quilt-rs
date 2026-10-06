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
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::MenuAction;
use crate::kit::Tooltip;
use crate::kit::icons;

/// An icon button that names its tooltip, which is the wiring every trigger
/// owes: the tooltip cannot reach into what the closure draws.
fn icon_trigger(label: &'static str, icon: fn() -> AnyView) -> impl FnOnce(String) -> AnyView {
    move |id| {
        view! {
            <IconButton
                icon=icon()
                aria_label=label
                variant=IconButtonVariant::Invisible
                on_click=|_| ()
                aria_describedby=id
            />
        }
        .into_any()
    }
}

fn hint(words: &'static str) -> Signal<String> {
    Signal::stored(words.to_string())
}

const NOTE: &str = "A sentence that explains a mark, and nothing else: plain text, the \
                    platform's own popover, and never the only place a fact lives. Nothing \
                    here is open — rest the pointer on a trigger for half a second, or Tab to \
                    one; a click's focus does not open it. Move onto the words and they stay; \
                    move away and they go a moment later. Escape, a click, a scroll or a \
                    resize closes it. The icon buttons still carry their native `title` as \
                    their name, which shows after a second beside the tooltip — moving names \
                    into tooltips is a separate change.";

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
                    trigger=icon_trigger("Refresh", icons::sync)
                />
            </Cell>
            <Cell label="beside an open menu — open the dots, then hover the other button; the menu stays">
                <div class="g-inline">
                    <ActionMenu aria_label="More actions for this file" actions=menu />
                    <Tooltip
                        text=hint("Opening this sentence closes no menu: it is a manual popover.")
                        trigger=icon_trigger("Refresh", icons::sync)
                    />
                </div>
            </Cell>
            <Cell full=true label="at the right edge — hangs leftwards from its trigger (Align::End)">
                <div class="g-inline g-inline--end" style="width:100%">
                    <Tooltip
                        text=hint("A trailing trigger lines up its right edge, so the words stay beside it at any window width.")
                        align=Align::End
                        trigger=icon_trigger("More", icons::overflow)
                    />
                </div>
            </Cell>
            <Cell full=true label="near the bottom — scroll until this row touches the window's bottom edge, then hover: it opens above">
                <Tooltip
                    text=hint("Below by preference, above when below does not fit.")
                    trigger=icon_trigger("Settings", icons::gear)
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
                                        trigger=icon_trigger("About this row", icons::diff)
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

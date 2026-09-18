//! `ActionMenu` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ActionMenu;
use crate::kit::MenuAction;

fn noop() -> Callback<()> {
    Callback::new(|()| ())
}

#[component]
pub fn ActionMenuStories() -> impl IntoView {
    view! {
        <Story
            title="ActionMenu"
            note="Select picks a value; ActionMenu fires a command — and there is no native \
                  element for `run one of these`. Open one and Tab through it: these are \
                  buttons in reading order, with no `role=menu`, because that role promises \
                  arrow keys and hand-writing roving focus is the thing the design forbids. \
                  A disabled command carries its reason rather than a flag. The destructive \
                  one sits below a rule and takes a confirmation afterwards."
        >
            <Cell wide=true label="a file's row menu">
                <ActionMenu
                    aria_label="More actions for this file"
                    actions=vec![
                        MenuAction::new("Open file", noop()),
                        MenuAction::new("Open in catalog", noop()),
                        MenuAction::new("Copy URI", noop()),
                        MenuAction::new("Ignore", noop()),
                        MenuAction::new("Stop keeping", noop()).danger(),
                    ]
                />
            </Cell>
            <Cell wide=true label="a command that cannot run, and why">
                <ActionMenu
                    aria_label="More package actions"
                    actions=vec![
                        MenuAction::new("Open folder", noop()),
                        MenuAction::new("Undo last publish", noop())
                            .disabled("Nothing has been published from this computer."),
                        MenuAction::new("Stop keeping", noop()).danger(),
                    ]
                />
            </Cell>
            <Cell wide=true label="one command — still a menu, not a button">
                <ActionMenu
                    aria_label="More actions"
                    actions=vec![MenuAction::new("Copy URI", noop())]
                />
            </Cell>
        </Story>
    }
}

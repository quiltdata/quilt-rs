//! A menu of commands, behind a trigger.
//!
//! # `ActionMenu`, not `Menu`
//!
//! The name carries the sentence that makes it legal under DESIGN.md's **Platform
//! Owns The Keyboard Rule**:
//!
//! > [`Select`](super::Select) picks a **value**. `ActionMenu` fires a
//! > **command**.
//!
//! The rule bans a listbox and a combobox because each replaces a native form
//! control that already works. A command menu replaces nothing — there is no
//! native element for *run one of these* — and it is not a form control, so
//! nothing it does can disagree with a `<select>`.
//!
//! # No `role="menu"`, deliberately
//!
//! `role="menu"` promises arrow-key navigation, and hand-writing roving focus is
//! the specific thing the rule forbids. These are buttons on a surface, reached
//! with Tab, in the order they are read. The promise and the behaviour match.
//!
//! # A disabled command says why
//!
//! [`MenuAction::disabled`] carries the reason rather than a flag. A greyed
//! command with no explanation is the one people file bugs about, and the
//! component makes the explanation the only way to grey it.

use leptos::prelude::*;

use super::AnchoredOverlay;
use super::IconButton;
use super::IconButtonVariant;
use super::icons;

stylance::import_crate_style!(style, "src/kit/action_menu.module.scss");

/// How loudly a command is offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionTone {
    #[default]
    Default,
    /// Destroys something. Never the first item, and always followed by a
    /// confirmation — the menu picks the command, the dialog accepts the
    /// consequence.
    Danger,
}

/// One command.
#[derive(Clone)]
pub struct MenuAction {
    pub label: String,
    pub tone: ActionTone,
    /// `Some(reason)` disables it **and states why**, as the item's `title` and
    /// its accessible description.
    pub disabled: Option<String>,
    /// Runs the command. The menu closes first, so a command that opens a dialog
    /// does not leave a surface floating over it.
    pub on_select: Callback<()>,
    /// Draws a rule above this item. The destructive commands sit below one.
    pub separated: bool,
}

impl MenuAction {
    #[must_use]
    pub fn new(label: impl Into<String>, on_select: Callback<()>) -> Self {
        Self {
            label: label.into(),
            tone: ActionTone::Default,
            disabled: None,
            on_select,
            separated: false,
        }
    }

    /// Destructive, below a rule. The two travel together everywhere this is
    /// used, so they are one call rather than two.
    #[must_use]
    pub fn danger(mut self) -> Self {
        self.tone = ActionTone::Danger;
        self.separated = true;
        self
    }

    /// Unavailable, and the reason the reader gets.
    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled = Some(reason.into());
        self
    }
}

#[component]
pub fn ActionMenu(
    /// Names the trigger, and through it the surface — `More actions for this
    /// file`, not `More`.
    #[prop(into)]
    aria_label: String,
    actions: Vec<MenuAction>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let surface_label = aria_label.clone();

    let trigger = view! {
        <IconButton
            icon=icons::overflow()
            aria_label=aria_label
            variant=IconButtonVariant::Invisible
            on_click=move |_| open.update(|o| *o = !*o)
        />
    }
    .into_any();

    let items = actions
        .into_iter()
        .map(|action| {
            let MenuAction {
                label,
                tone,
                disabled,
                on_select,
                separated,
            } = action;
            let title = disabled.clone();
            let reason = disabled.clone();
            let is_disabled = disabled.is_some();

            let mut class = String::from(style::item);
            if matches!(tone, ActionTone::Danger) {
                class.push(' ');
                class.push_str(style::danger);
            }
            if separated {
                class.push(' ');
                class.push_str(style::separated);
            }

            view! {
                <button
                    type="button"
                    class=class
                    title=title
                    disabled=is_disabled
                    on:click=move |_| {
                        // Closed first: a command that opens a dialog must not
                        // leave this floating over it in the top layer.
                        open.set(false);
                        on_select.run(());
                    }
                >
                    {label}
                    {reason
                        .map(|why| view! { <span class=style::reason>{why}</span> })}
                </button>
            }
        })
        .collect_view();

    view! {
        <AnchoredOverlay trigger=trigger open=open aria_label=surface_label>
            <div class=style::list>{items.clone()}</div>
        </AnchoredOverlay>
    }
}

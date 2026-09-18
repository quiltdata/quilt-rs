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
//! # No `role="menu"`, and no `aria-haspopup`, deliberately
//!
//! `role="menu"` promises arrow-key navigation, and hand-writing roving focus is
//! the specific thing the rule forbids. These are buttons on a surface, reached
//! with Tab, in the order they are read. The promise and the behaviour match.
//!
//! `aria-haspopup` goes the same way: its `true` is defined as synonymous with
//! `menu`, so setting it would make exactly the promise the paragraph above
//! declines. What the trigger *does* carry is `aria-expanded` and
//! `aria-controls` — it says that it opens something, which of it is open, and
//! which surface it means, none of which claim a keyboard model.
//!
//! # It hangs leftwards, and that is not a prop
//!
//! The trigger is an overflow glyph, which is a trailing control everywhere it is
//! used — the end of a row, the end of a header. So the surface always aligns its
//! right edge to the trigger's and grows left. Left-aligning instead would push it
//! into [`AnchoredOverlay`]'s viewport clamp, which pins the surface to the window
//! rather than to the button that opened it. If a leading `[⋯]` ever exists, this
//! becomes a prop; inventing one for a caller that does not exist would not.
//!
//! # A disabled command says why
//!
//! [`MenuAction::disabled`] carries the reason rather than a flag. A greyed
//! command with no explanation is the one people file bugs about, and the
//! component makes the explanation the only way to grey it.

use leptos::prelude::*;

use super::Align;
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

    let trigger = move |surface_id: String| {
        view! {
            <IconButton
                icon=icons::overflow()
                aria_label=aria_label
                variant=IconButtonVariant::Invisible
                aria_expanded=open
                aria_controls=surface_id
                on_click=move |_| open.update(|o| *o = !*o)
            />
        }
        .into_any()
    };

    let items = surface(actions, open);

    view! {
        <AnchoredOverlay
            trigger=trigger
            open=open
            aria_label=surface_label
            align=Align::End
            tight=true
        >
            {items}
        </AnchoredOverlay>
    }
}

/// The list of commands, as it is drawn inside an [`AnchoredOverlay`].
///
/// Shared with [`SplitButton`](super::SplitButton), which is the same list
/// behind a different trigger. Extracted rather than written twice: two copies
/// of the separator rule and the close-then-run order would drift, and this file
/// already owns the stylesheet they are drawn with.
pub(super) fn surface(actions: Vec<MenuAction>, open: RwSignal<bool>) -> AnyView {
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

            view! {
                // Its own element, not a border on the item below it. As an edge
                // it could only be spaced from one side — the item above already
                // has its own padding, so the rule sat closer to the command
                // under it than to the one over it. A separator owns the space on
                // both sides, and every item keeps identical padding.
                {separated
                    .then(|| view! { <div class=style::separator role="separator" /> })}
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

    view! { <div class=style::list>{items}</div> }.into_any()
}

/// The options of a [`SplitButton`](super::SplitButton), as they are drawn
/// inside an [`AnchoredOverlay`].
///
/// Separate from [`surface`] because the two menus mean different things.
/// `ActionMenu`'s items are commands: each runs and the menu closes. These are
/// **choices**: picking one moves the mark and changes what the face will do,
/// and nothing runs until the face itself is clicked. Sharing one function would
/// mean a parameter that silently changes what a click does.
///
/// `aria-current` rather than `aria-checked`: the latter needs a `radio` or
/// `menuitemradio` role, and both promise the arrow-key model this kit
/// deliberately does not hand-write — the same reason `ActionMenu` declines
/// `role="menu"`. `aria-current` states which one is active and claims nothing
/// about how to move between them.
///
/// **`current` and `selected` are two different things and must stay so.**
/// `current` is what the face is showing, already normalised; `selected` is the
/// caller's raw store, which a stale preference can put out of range. Marking
/// from the raw value leaves every option unticked while the face shows one of
/// them — the menu saying "none of these" about a button that is about to run.
pub(super) fn choices(
    labels: Vec<String>,
    current: Signal<usize>,
    selected: RwSignal<usize>,
    open: RwSignal<bool>,
) -> AnyView {
    let items = labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let class = format!("{} {}", style::item, style::choice);
            let is_current = move || current.get() == index;
            view! {
                <button
                    type="button"
                    class=class
                    aria-current=move || is_current().then_some("true")
                    on:click=move |_| {
                        // Sets the default; it does not run it. Opening a menu to
                        // change a preference must not also publish.
                        selected.set(index);
                        open.set(false);
                    }
                >
                    {move || {
                        if is_current() {
                            view! { <span class=style::mark>{icons::check()}</span> }.into_any()
                        } else {
                            view! { <span class=style::unmarked /> }.into_any()
                        }
                    }}
                    {label}
                </button>
            }
        })
        .collect_view();

    view! { <div class=style::list>{items}</div> }.into_any()
}

//! `FormDialog`, `TextInput`, `FormControl`, and the forms the package page opens.
//!
//! Each form is a component rendered **twice**: inline in a cell, so it can be reviewed at
//! a glance and screenshotted, and inside a real modal behind a button, so the parts only
//! a modal has — the focus trap, Escape, the backdrop — can actually be exercised. One
//! definition, so the two cannot drift.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::FormControl;
use crate::kit::FormDialog;
use crate::kit::Naming;
use crate::kit::Select;
use crate::kit::Submit;
use crate::kit::TextInput;

#[component]
pub fn FormsStories() -> impl IntoView {
    view! { <Inputs /> <FormControls /> <FormBodies /> <Submitting /> }
}

#[component]
fn Inputs() -> impl IntoView {
    let empty = RwSignal::new(String::new());
    let filled = RwSignal::new("user/plate-07".to_string());
    let bad = RwSignal::new("user / plate 07".to_string());
    let locked = RwSignal::new("open.quiltdata.com".to_string());

    view! {
        <Story
            title="TextInput"
            note="Separate from SearchInput: that has a clear button and a value you expect \
                  to throw away, while this is a value being entered and can be invalid. \
                  Invalid draws the border and nothing else — a red fill behind text \
                  somebody is still fixing makes it harder to read. Every cell is wrapped in \
                  a FormControl, because a bare TextInput does not compile: it demands a \
                  `ControlId`, and FormControl is the only source of one."
        >
            <Cell wide=true label="empty, with a placeholder">
                <FormControl
                    label="Package name"
                    control=move |id| {
                        view! { <TextInput id=id value=empty placeholder="owner/package-name" /> }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="with a value">
                <FormControl
                    label="Package name"
                    control=move |id| {
                        view! {
                            <TextInput id=id value=filled placeholder="owner/package-name" />
                        }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="invalid — border only">
                <FormControl
                    label="Package name"
                    control=move |id| {
                        view! {
                            <TextInput
                                id=id
                                value=bad
                                placeholder="owner/package-name"
                                invalid=true
                            />
                        }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="disabled — showing a value you may not change">
                <FormControl
                    label="Host"
                    control=move |id| {
                        view! { <TextInput id=id value=locked disabled=true /> }.into_any()
                    }
                />
            </Cell>
        </Story>
    }
}

#[component]
fn FormControls() -> impl IntoView {
    let plain = RwSignal::new(String::new());
    let captioned = RwSignal::new(String::new());
    let required = RwSignal::new(String::new());
    let broken = RwSignal::new("my bucket".to_string());

    view! {
        <Story
            title="FormControl"
            note="FormControl hands its control the ids it allocated, through a closure. \
                  `ControlId`'s constructor is private and every form control demands one, \
                  so an unlabelled control is a compile error rather than an audit finding — \
                  we shipped exactly that bug once, when Select's label rendered nowhere. \
                  The caption and the message are `aria-describedby`, so they are announced; \
                  inside a label they would have become part of the name. Required says the \
                  word rather than drawing an asterisk."
        >
            <Cell wide=true label="label only">
                <FormControl
                    label="Bucket"
                    control=move |id| {
                        view! { <TextInput id=id value=plain placeholder="my-s3-bucket" /> }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="with a caption — announced, via aria-describedby">
                <FormControl
                    label="Package name"
                    caption="Two parts, separated by a slash — user/plate-07."
                    control=move |id| {
                        view! {
                            <TextInput id=id value=captioned placeholder="owner/package-name" />
                        }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="required">
                <FormControl
                    label="Bucket"
                    required=true
                    control=move |id| {
                        view! { <TextInput id=id value=required placeholder="my-s3-bucket" /> }
                            .into_any()
                    }
                />
            </Cell>
            <Cell wide=true label="in error — border here, reason below, both announced">
                <FormControl
                    label="Bucket"
                    error="Bucket names cannot contain spaces."
                    control=move |id| {
                        view! { <TextInput id=id value=broken invalid=true /> }.into_any()
                    }
                />
            </Cell>
        </Story>
    }
}

/// The bucket form, in both of its shapes.
///
/// `locked` is not a spare state: a package that has been pushed is pinned to its push
/// history, so the header's command reads `Show remote` and the same three fields are
/// shown rather than offered. The v2 header already makes that swap.
#[component]
pub fn BucketForm(
    /// Every field disabled — the remote as a fact rather than a choice.
    #[prop(optional)]
    locked: bool,
) -> impl IntoView {
    let host = RwSignal::new("open.quiltdata.com".to_string());
    // Locked means *showing a remote*, so there is one to show. Left empty it drew a grey
    // placeholder, and a read-only form whose field is a placeholder reads as a value the
    // reader cannot change rather than as the absence of one.
    let bucket = RwSignal::new(if locked {
        "quilt-example".to_string()
    } else {
        String::new()
    });
    let workflow = RwSignal::new("Default".to_string());

    view! {
        // Host and Bucket are named concretely on purpose. The vocabulary bans naming the
        // platform as an abstraction — no "…from Quilt" — but explicitly allows concrete
        // endpoints "where the user actually chooses or authenticates against one", and
        // this is that place.
        <FormControl
            label="Host"
            // A caption instructs while the field is a choice and states once it is not.
            // "From your accounts" tells a reader where to get a value they cannot enter.
            caption=if locked {
                "Where this package is published."
            } else {
                "Where this package is published. From your accounts."
            }
            control=move |id| {
                view! {
                    <TextInput
                        id=id
                        value=host
                        placeholder="open.quiltdata.com"
                        disabled=locked
                    />
                }
                    .into_any()
            }
        />
        // Two shapes rather than one with flags, because they differ in what they say and
        // not only in whether they accept typing. Nothing is *required* of a reader who
        // cannot type — `(required)` on a disabled field is the kind of thing only a
        // screenshot catches — and the caption that replaces it is why the field is grey,
        // which is otherwise a mystery the reader has to guess at.
        {if locked {
            view! {
                <FormControl
                    label="Bucket"
                    caption="Fixed by this package's push history."
                    control=move |id| {
                        view! { <TextInput id=id value=bucket disabled=true /> }.into_any()
                    }
                />
            }
                .into_any()
        } else {
            view! {
                <FormControl
                    label="Bucket"
                    required=true
                    control=move |id| {
                        view! { <TextInput id=id value=bucket placeholder="my-s3-bucket" /> }
                            .into_any()
                    }
                />
            }
                .into_any()
        }}
        // Read from the bucket once it is known, so it is last and its options depend on
        // the field above it.
        //
        // `Naming::FormControl`, so the Select renders no name of its own. It used to be given
        // `label="Workflow"` here as well as by the FormControl, which nested two labels and named
        // the control twice — the kind of thing the ControlId design exists to make unsayable.
        <FormControl
            label="Workflow"
            caption="Rules the bucket applies when you publish."
            control=move |id| {
                view! {
                    <Select
                        naming=Naming::FormControl(id)
                        options=vec!["Default".to_string(), "None".to_string()]
                        selected=workflow
                        disabled=locked
                    />
                }
                    .into_any()
            }
        />
    }
}

/// The create form. Local-only: a package with no bucket yet, which is exactly the state
/// the queue reports as `No S3 bucket yet`.
#[component]
pub fn CreateForm() -> impl IntoView {
    let name = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());

    view! {
        <FormControl
            label="Package name"
            caption="Two parts, separated by a slash — user/plate-07."
            required=true
            control=move |id| {
                view! {
                    <TextInput
                        id=id
                        value=name
                        placeholder="owner/package-name"
                        autofocus=true
                    />
                }
                    .into_any()
            }
        />
        // A path the user picks with the OS dialog rather than types, so the text field is
        // disabled and the Browse button is the control. Optional: an empty package is a
        // legitimate starting point, and the vocabulary says so by not calling this
        // "source".
        <FormControl
            label="Folder to add"
            caption="Optional. You can add files later."
            control=move |id| {
                view! {
                    <div class="g-inline">
                        <TextInput
                            id=id
                            value=folder
                            placeholder="No folder chosen"
                            disabled=true
                        />
                        <Button on_click=move |_| {
                            folder.set("/home/you/runs/plate-07".to_string());
                        }>"Browse…"</Button>
                    </div>
                }
                    .into_any()
            }
        />
    }
}

// ── FormDialog ──

/// A stand-in for a command. The delay is the point: an action that answers instantly
/// never shows the in-flight footer it is here to demonstrate.
async fn after_a_beat(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        drop(
            web_sys::window()
                .expect("a window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms),
        );
    });
    drop(wasm_bindgen_futures::JsFuture::from(promise).await);
}

/// One case, as the button that opens it. A `FormDialog` cannot be drawn inline —
/// `show_modal()` puts it in the top layer over the whole page — so the only honest way
/// to review a state is to open it.
#[component]
fn Case(
    title: &'static str,
    submit_label: &'static str,
    /// `Some(reason)` makes the action fail with it; `None` makes it succeed.
    #[prop(optional)]
    rejects: Option<&'static str>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <Button on_click=move |_| open.set(true)>{title}</Button>
        <FormDialog
            open=open
            title=title
            submit=Submit::new(
                submit_label,
                move || async move {
                    after_a_beat(700).await;
                    match rejects {
                        Some(reason) => Err(reason.to_string()),
                        None => Ok(()),
                    }
                },
            )
        >
            <BucketForm />
        </FormDialog>
    }
}

/// The read-only shape: no submit, so no form and one way out.
#[component]
fn ShowRemoteCase() -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <Button on_click=move |_| open.set(true)>"Show remote"</Button>
        <FormDialog open=open title="Show remote">
            <BucketForm locked=true />
        </FormDialog>
    }
}

#[component]
fn Submitting() -> impl IntoView {
    view! {
        <Story
            title="FormDialog"
            note="Open one and press Return in a field — it submits, which none of v1's \
                  four overlays does. Watch the footer while it runs: the primary goes \
                  busy and Cancel is refused, because dismissing would not stop a write \
                  already in flight. A refusal stays open with the reason above the \
                  fields, where v1 put it in the page's slot behind the modal."
        >
            <Cell label="it saves">
                <Case title="Change bucket" submit_label="Save" />
            </Cell>
            <Cell label="it is refused">
                <Case
                    title="Change bucket"
                    submit_label="Save"
                    rejects="No permission to write to quilt-example."
                />
            </Cell>
            <Cell label="nothing to submit — pushed">
                <ShowRemoteCase />
            </Cell>
        </Story>
    }
}

#[component]
fn FormBodies() -> impl IntoView {
    view! {
        <Story
            title="The two forms, inline"
            note="Rendered here for review and again inside real modals in the scene below \
                  — one definition each, so they cannot drift. \
                  \
                  Both replace hand-rolled overlays in v1. The labels are the part NOT yet \
                  settled by the vocabulary spec, which fixed the action names but never \
                  the field names: v1 says Namespace and Source directory, and both are \
                  jargon by the spec's own test, so these read Package name and \
                  Folder to add. Worth a decision rather than inheritance."
        >
            <Cell wide=true label="Choose S3 bucket — v1's Set remote">
                <div class="g-bars">
                    <BucketForm />
                </div>
            </Cell>
            <Cell wide=true label="Create package — local only, no bucket yet">
                <div class="g-bars">
                    <CreateForm />
                </div>
            </Cell>
        </Story>
    }
}

/// The refusal, at rest — and the real thing one click away.
///
/// Both, because they cannot be the same object. A `FormDialog` is modal: `show_modal()`
/// puts it in the top layer and its backdrop takes every pointer event on the page, so a
/// scene holding one open makes the rest of the gallery unclickable. Driving the real
/// component into this state and leaving it there was tried and does exactly that.
///
/// So the resting copy is composed inline from the same kit pieces the dialog uses —
/// `Banner`, the form, the footer's two buttons — which is how `StripErrorScene` shows a
/// strip that could not load, and what this module's own doc means by rendering a form
/// twice. The button beside it opens the real one, whose banner arrives the real way.
#[component]
pub fn RefusedScene() -> impl IntoView {
    let live = RwSignal::new(false);
    let dismissed = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · a dialog that was refused"
            note="Where v1 puts this: the page's notification slot, behind the modal that \
                  caused it. Here it is above the fields, the values are still there to \
                  correct, and the reason names the bucket. This copy is inline, at the \
                  width the modal gets, so it can be read and screenshotted without \
                  taking the page's pointer events — press Open the real one for the \
                  modal, whose banner arrives through an actual refused submit."
        >
            <div class="g-bars g-dialog-inline">
                <Show when=move || !dismissed.get()>
                    <Banner
                        variant=BannerVariant::Critical
                        on_dismiss=move |_| dismissed.set(true)
                    >
                        "No permission to write to quilt-example."
                    </Banner>
                </Show>
                <BucketForm />
                // The footer, in the dialog's own arrangement: right-aligned, primary last.
                <div class="g-inline g-inline--end">
                    <Button on_click=move |_| ()>"Cancel"</Button>
                    <Button variant=ButtonVariant::Primary on_click=move |_| ()>"Save"</Button>
                </div>
            </div>
            <div class="g-inline">
                <Button on_click=move |_| live.set(true)>"Open the real one"</Button>
            </div>
            <FormDialog
                open=live
                title="Change bucket"
                submit=Submit::new(
                    "Save",
                    || async { Err("No permission to write to quilt-example.".to_string()) },
                )
            >
                <BucketForm />
            </FormDialog>
        </Scene>
    }
}

#[component]
pub fn DialogScene() -> impl IntoView {
    let bucket_open = RwSignal::new(false);
    let create_open = RwSignal::new(false);
    let show_open = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · the three dialogs"
            note="The two forms and the read-only one, as the page opens them. Tab: focus \
                  is trapped, which none of v1's four overlays does. Escape closes, which \
                  none of them handles. The backdrop is the platform's top layer, so there \
                  is no z-index and no ancestor can clip it — and clicking it does not \
                  close them, deliberately: each holds a form, and a stray click \
                  discarding what you typed is a bad trade for saving a movement to Cancel."
        >
            <div class="g-inline">
                <Button on_click=move |_| bucket_open.set(true)>"Change bucket"</Button>
                <Button on_click=move |_| show_open.set(true)>"Show remote"</Button>
                <Button variant=ButtonVariant::Primary on_click=move |_| create_open.set(true)>
                    "Create package"
                </Button>
            </div>
            <FormDialog
                open=bucket_open
                title="Change bucket"
                submit=Submit::new(
                    "Save",
                    move || async move {
                        after_a_beat(700).await;
                        Ok(())
                    },
                )
            >
                <BucketForm />
            </FormDialog>
            // A pushed package is pinned to its push history: the same fields, shown and
            // not offered. The header swaps the command's label to match.
            <FormDialog open=show_open title="Show remote">
                <BucketForm locked=true />
            </FormDialog>
            <FormDialog
                open=create_open
                title="Create package"
                submit=Submit::new(
                    "Create",
                    move || async move {
                        after_a_beat(700).await;
                        Ok(())
                    },
                )
            >
                <CreateForm />
            </FormDialog>
        </Scene>
    }
}

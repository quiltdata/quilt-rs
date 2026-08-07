//! `Dialog`, `TextInput`, `Field`, and the two forms the main page opens.
//!
//! Each form is a component rendered **twice**: inline in a cell, so it can be reviewed at
//! a glance and screenshotted, and inside a real modal behind a button, so the parts only
//! a modal has — the focus trap, Escape, the backdrop — can actually be exercised. One
//! definition, so the two cannot drift.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Dialog;
use crate::kit::Field;
use crate::kit::Select;
use crate::kit::TextInput;

#[component]
pub fn FormsStories() -> impl IntoView {
    view! { <Inputs /> <Fields /> <FormBodies /> }
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
            note="Separate from SearchInput rather than a variant of it: a search field has \
                  a clear button, type=search, and a value the user expects to throw away, \
                  while this is a value being ENTERED and can be invalid. \
                  \
                  Invalid draws the border and nothing else. A red fill behind text the \
                  user is still typing makes it harder to read at the moment they are \
                  trying to fix it — and the border plus Field's message already say it \
                  twice. It carries no label; Field does."
        >
            <Cell label="empty, with a placeholder">
                <TextInput value=empty placeholder="owner/package-name" />
            </Cell>
            <Cell label="with a value">
                <TextInput value=filled placeholder="owner/package-name" />
            </Cell>
            <Cell label="invalid — border only">
                <TextInput value=bad placeholder="owner/package-name" invalid=true />
            </Cell>
            <Cell label="disabled — showing a value you may not change">
                <TextInput value=locked disabled=true />
            </Cell>
        </Story>
    }
}

#[component]
fn Fields() -> impl IntoView {
    let plain = RwSignal::new(String::new());
    let captioned = RwSignal::new(String::new());
    let required = RwSignal::new(String::new());
    let broken = RwSignal::new("my bucket".to_string());

    view! {
        <Story
            title="Field"
            note="The label is a real <label> WRAPPING the control, so the browser \
                  associates them with no for, no id, and nothing to keep in step. \
                  \
                  Which is also why the caption and the error are OUTSIDE it: everything \
                  inside a label contributes to the accessible name, and \
                  'Package name owner/package-name Use owner/name' is a worse name than \
                  'Package name'. Outside, they are visually associated and NOT announced \
                  — there is no aria-describedby, because that needs ids on both ends. \
                  That is the concrete cost of deferring qhq-kt31, and its concrete reason. \
                  \
                  The error carries the Danger tone's own glyph, so it agrees with every \
                  other red thing on the page and survives greyscale. Required says the \
                  word rather than showing an asterisk, which is a convention you have to \
                  have learned."
        >
            <Cell wide=true label="label only">
                <Field label="Bucket">
                    <TextInput value=plain placeholder="my-s3-bucket" />
                </Field>
            </Cell>
            <Cell wide=true label="with a caption">
                <Field label="Package name" caption="Two parts, separated by a slash — user/plate-07.">
                    <TextInput value=captioned placeholder="owner/package-name" />
                </Field>
            </Cell>
            <Cell wide=true label="required">
                <Field label="Bucket" required=true>
                    <TextInput value=required placeholder="my-s3-bucket" />
                </Field>
            </Cell>
            <Cell wide=true label="in error — border here, reason below">
                <Field label="Bucket" error="Bucket names cannot contain spaces.">
                    <TextInput value=broken invalid=true />
                </Field>
            </Cell>
        </Story>
    }
}

/// The bucket form. `Choose S3 bucket` per the settled vocabulary — v1 calls this
/// `Set remote`, which names a concept rather than the thing you pick.
#[component]
pub fn BucketForm() -> impl IntoView {
    let host = RwSignal::new("open.quiltdata.com".to_string());
    let bucket = RwSignal::new(String::new());
    let workflow = RwSignal::new("Default".to_string());

    view! {
        // Host and Bucket are named concretely on purpose. The vocabulary bans naming the
        // platform as an abstraction — no "…from Quilt" — but explicitly allows concrete
        // endpoints "where the user actually chooses or authenticates against one", and
        // this is that place.
        <Field label="Host" caption="Where this package is published. From your accounts.">
            <TextInput value=host placeholder="open.quiltdata.com" />
        </Field>
        <Field label="Bucket" required=true>
            <TextInput value=bucket placeholder="my-s3-bucket" />
        </Field>
        // Read from the bucket once it is known, so it is last and its options depend on
        // the field above it.
        <Field label="Workflow" caption="Rules the bucket applies when you publish.">
            <Select
                label="Workflow"
                options=vec!["Default".to_string(), "None".to_string()]
                selected=workflow
            />
        </Field>
    }
}

/// The create form. Local-only: a package with no bucket yet, which is exactly the state
/// the queue reports as `No S3 bucket yet`.
#[component]
pub fn CreateForm() -> impl IntoView {
    let name = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());

    view! {
        <Field
            label="Package name"
            caption="Two parts, separated by a slash — user/plate-07."
            required=true
        >
            <TextInput value=name placeholder="owner/package-name" autofocus=true />
        </Field>
        // A path the user picks with the OS dialog rather than types, so the text field is
        // disabled and the Browse button is the control. Optional: an empty package is a
        // legitimate starting point, and the vocabulary says so by not calling this
        // "source".
        <Field label="Folder to add" caption="Optional. You can add files later.">
            <div class="g-inline">
                <TextInput value=folder placeholder="No folder chosen" disabled=true />
                <Button on_click=move |_| folder.set("/home/you/runs/plate-07".to_string())>
                    "Browse…"
                </Button>
            </div>
        </Field>
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

#[component]
pub fn DialogScene() -> impl IntoView {
    let bucket_open = RwSignal::new(false);
    let create_open = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · the two dialogs"
            note="OPEN THEM — the parts worth reviewing are the ones only a modal has. Tab: \
                  focus is trapped inside, which none of v1's four overlays does. Escape \
                  closes, which none of them handles. The backdrop is the platform's top \
                  layer, so there is no z-index and nothing can clip it — v1's \
                  div.popup-overlay could be clipped by any ancestor's overflow. \
                  \
                  Clicking the backdrop does NOT close them, deliberately unlike v1. Every \
                  one of these holds a form, and a stray click discarding what you typed is \
                  a bad trade for saving a movement to Cancel. \
                  \
                  This one <dialog> replaces four hand-rolled overlays: set_remote_popup, \
                  ignore_popup, workflow_select, and the create-package form inside \
                  installed_packages_list."
        >
            <div class="g-inline">
                <Button on_click=move |_| bucket_open.set(true)>"Choose S3 bucket"</Button>
                <Button variant=ButtonVariant::Primary on_click=move |_| create_open.set(true)>
                    "Create package"
                </Button>
            </div>
            <Dialog
                open=bucket_open
                title="Choose S3 bucket"
                footer=view! {
                    <Button on_click=move |_| bucket_open.set(false)>"Cancel"</Button>
                    <Button variant=ButtonVariant::Primary on_click=move |_| bucket_open.set(false)>
                        "Choose"
                    </Button>
                }
                    .into_any()
            >
                <BucketForm />
            </Dialog>
            <Dialog
                open=create_open
                title="Create package"
                footer=view! {
                    <Button on_click=move |_| create_open.set(false)>"Cancel"</Button>
                    <Button variant=ButtonVariant::Primary on_click=move |_| create_open.set(false)>
                        "Create"
                    </Button>
                }
                    .into_any()
            >
                <CreateForm />
            </Dialog>
        </Scene>
    }
}

//! `Dialog`, `TextInput`, `FormControl`, and the two forms the main page opens.
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
use crate::kit::FormControl;
use crate::kit::Naming;
use crate::kit::Select;
use crate::kit::TextInput;

#[component]
pub fn FormsStories() -> impl IntoView {
    view! { <Inputs /> <FormControls /> <FormBodies /> }
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
                  trying to fix it — and the border plus FormControl's message already say it \
                  twice. \
                  \
                  Every cell here is wrapped in a FormControl, because a bare TextInput no longer \
                  COMPILES: it demands a ControlId and the only source of one is FormControl's \
                  control closure. That is deliberate — see the FormControl story below."
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
            note="FormControl hands its control the ids it allocated, through a closure. That \
                  closure is the whole design: ControlId's constructor is PRIVATE, every \
                  control that belongs in a form demands one, so an unlabelled control is a \
                  compile error. Primer leaves that to eslint and axe in CI — we have \
                  neither, and we shipped exactly that bug once when Select's label rendered \
                  nowhere. \
                  \
                  The ids also buy what the previous wrapping-label version could not: the \
                  caption and the message are now aria-describedby, so they are ANNOUNCED. \
                  They could never be inside the label, because everything in a label \
                  becomes part of the name, and 'Package name owner/package-name Use \
                  owner/name' is a worse name than 'Package name'. \
                  \
                  The error carries the Danger tone's own glyph, so it agrees with every \
                  other red thing on the page and survives greyscale. Required says the \
                  word rather than showing an asterisk, which is a convention you have to \
                  have learned."
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
        <FormControl
            label="Host"
            caption="Where this package is published. From your accounts."
            control=move |id| {
                view! { <TextInput id=id value=host placeholder="open.quiltdata.com" /> }
                    .into_any()
            }
        />
        <FormControl
            label="Bucket"
            required=true
            control=move |id| {
                view! { <TextInput id=id value=bucket placeholder="my-s3-bucket" /> }.into_any()
            }
        />
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

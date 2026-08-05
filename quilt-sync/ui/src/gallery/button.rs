//! Button stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Button;
use crate::kit::ButtonSize;
use crate::kit::ButtonVariant;

/// Stand-in glyphs. A real icon set is a later component; these exist so the
/// leading slot can be reviewed now, including its collision with the spinner.
fn plus_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round">
            <path d="M8 3.5v9M3.5 8h9" />
        </svg>
    }
    .into_any()
}

fn download_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <path d="M8 2.5v7.5M4.75 7l3.25 3 3.25-3M2.5 13.5h11" />
        </svg>
    }
    .into_any()
}

#[component]
pub fn ButtonStories() -> impl IntoView {
    view! { <Plain /> <WithIcon /> <Large /> }
}

#[component]
fn Plain() -> impl IntoView {
    view! {
        <Story
            title="Button"
            note="Two variants. Hover and active are pointer states — point at the first \
                  two cells to review them. Disabled is not focusable; loading is."
        >
            <Cell label="default">
                <Button on_click=|_| ()>"Get latest"</Button>
            </Cell>
            <Cell label="default · disabled">
                <Button on_click=|_| () disabled=true>"Get latest"</Button>
            </Cell>
            <Cell label="default · loading">
                <Button on_click=|_| () loading=true>"Checking\u{2026}"</Button>
            </Cell>
            <Cell label="default · long label">
                <Button on_click=|_| ()>"Choose S3 bucket for this package"</Button>
            </Cell>

            <Cell label="primary">
                <Button on_click=|_| () variant=ButtonVariant::Primary>"Publish"</Button>
            </Cell>
            <Cell label="primary · disabled">
                <Button on_click=|_| () variant=ButtonVariant::Primary disabled=true>
                    "Publish"
                </Button>
            </Cell>
            <Cell label="primary · loading">
                <Button on_click=|_| () variant=ButtonVariant::Primary loading=true>
                    "Publishing\u{2026}"
                </Button>
            </Cell>
            <Cell label="primary · long label">
                <Button on_click=|_| () variant=ButtonVariant::Primary>
                    "Publish your changes to s3://vir-quilt-res-3-in-progress"
                </Button>
            </Cell>

            <Cell label="loading implies disabled — setting both changes nothing">
                <Button on_click=|_| () loading=true disabled=true>"Retry"</Button>
            </Cell>
            <Cell label="a pair, as the queue uses them">
                <div class="g-inline">
                    <Button on_click=|_| () variant=ButtonVariant::Primary>"Resolve"</Button>
                    <Button on_click=|_| ()>"Dismiss"</Button>
                </div>
            </Cell>
        </Story>
    }
}

#[component]
fn WithIcon() -> impl IntoView {
    view! {
        <Story
            title="Button · with icon"
            note="The icon and the loading spinner are ONE slot, never two. Compare \
                  `icon · primary` with `icon · loading`: the spinner replaces the icon, so \
                  the width does not move. A button with no icon does grow when loading \
                  starts — see the main section — which is accepted because callers swap \
                  the label at the same moment anyway."
        >
            <Cell label="icon + label">
                <Button on_click=|_| () icon=plus_icon()>"Create package"</Button>
            </Cell>
            <Cell label="icon · primary">
                <Button on_click=|_| () variant=ButtonVariant::Primary icon=download_icon()>
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="icon · loading — spinner replaces the icon">
                <Button
                    on_click=|_| ()
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                    loading=true
                >
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="icon · disabled">
                <Button on_click=|_| () icon=plus_icon() disabled=true>"Create package"</Button>
            </Cell>
            <Cell label="icon · long label">
                <Button on_click=|_| () icon=download_icon()>
                    "Get latest revision of this package"
                </Button>
            </Cell>
        </Story>
    }
}

#[component]
fn Large() -> impl IntoView {
    view! {
        <Story
            title="Button · large"
            note="One step up, for page-level and dialog-confirm actions. Size is \
                  orthogonal to variant, so every weight is available at both sizes, and \
                  the leading slot scales with it."
        >
            <Cell label="large">
                <Button on_click=|_| () size=ButtonSize::Large>"Create package"</Button>
            </Cell>
            <Cell label="large · primary">
                <Button on_click=|_| () size=ButtonSize::Large variant=ButtonVariant::Primary>
                    "Publish"
                </Button>
            </Cell>
            <Cell label="large · disabled">
                <Button on_click=|_| () size=ButtonSize::Large disabled=true>
                    "Create package"
                </Button>
            </Cell>
            <Cell label="large · icon">
                <Button on_click=|_| () size=ButtonSize::Large icon=plus_icon()>
                    "Create package"
                </Button>
            </Cell>
            <Cell label="large · primary · icon">
                <Button
                    on_click=|_| ()
                    size=ButtonSize::Large
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                >
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="large · loading — slot scales to 16px">
                <Button
                    on_click=|_| ()
                    size=ButtonSize::Large
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                    loading=true
                >
                    "Publishing\u{2026}"
                </Button>
            </Cell>
        </Story>
    }
}

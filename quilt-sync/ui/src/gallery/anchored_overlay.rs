//! `AnchoredOverlay` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::AnchoredOverlay;
use crate::kit::Button;
use crate::kit::LoadFailure;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::SkeletonBox;

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// Every trigger owes `aria-expanded` and `aria-controls`; the overlay hands the
/// id over precisely so a caller can wire them.
fn trigger(label: &'static str, open: RwSignal<bool>) -> impl FnOnce(String) -> AnyView {
    move |surface_id| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                {label}
            </Button>
        }
        .into_any()
    }
}

#[component]
pub fn AnchoredOverlayStories() -> impl IntoView {
    let loaded = RwSignal::new(false);
    let pending = RwSignal::new(false);
    let failed = RwSignal::new(false);
    let empty = RwSignal::new(false);

    view! {
        <Story
            title="AnchoredOverlay"
            note="Click a trigger. The cells hold only the buttons, because the surface \
                  opens into the top layer and floats over the page rather than sitting in \
                  its cell. Open one and press Escape, open one and click away, open one and \
                  scroll: light dismiss, Escape and the top layer are the platform's, and a \
                  surface that has lost its anchor closes. Only the position is ours, until \
                  CSS anchor positioning reaches WebKit. It opens immediately into whatever \
                  it has and fills when the data lands; the last cell is a LoadFailure."
        >
            <Cell wide=true label="click it — the revisions this copy holds">
                <AnchoredOverlay
                    trigger=trigger("Revisions you have (3)", loaded)
                    open=loaded
                    aria_label="Revisions you have"
                >
                    <PaneSection>
                        <RevisionRow message="Add Ernest thread" at=ago(2.0 * HOUR) />
                        <RevisionRow message="Add Caihong folder-upload note" at=ago(3.0 * DAY) />
                        <RevisionRow message="" at=ago(9.0 * DAY) />
                    </PaneSection>
                </AnchoredOverlay>
            </Cell>
            <Cell wide=true label="click it — pending, opens first and fills after">
                <AnchoredOverlay
                    trigger=trigger("Revisions you have (3)", pending)
                    open=pending
                    aria_label="Revisions you have"
                >
                    <PaneSection>
                        <SkeletonBox width="220px" />
                        <SkeletonBox width="180px" />
                        <SkeletonBox width="200px" />
                    </PaneSection>
                </AnchoredOverlay>
            </Cell>
            <Cell wide=true label="click it — a Tauri call that did not return">
                <AnchoredOverlay
                    trigger=trigger("Revisions you have", failed)
                    open=failed
                    aria_label="Revisions you have"
                >
                    <LoadFailure
                        words="Could not load your revisions."
                        on_retry=Callback::new(|()| ())
                    />
                </AnchoredOverlay>
            </Cell>
            <Cell wide=true label="click it — one revision, so callers hide the trigger instead">
                <AnchoredOverlay
                    trigger=trigger("Revisions you have (1)", empty)
                    open=empty
                    aria_label="Revisions you have"
                >
                    <PaneSection>
                        <RevisionRow message="Add Ernest thread" at=ago(2.0 * HOUR) />
                    </PaneSection>
                </AnchoredOverlay>
            </Cell>
        </Story>
    }
}

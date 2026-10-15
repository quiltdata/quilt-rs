//! `NameTooltip` stories.
//!
//! Nothing is open, for `Tooltip`'s reason: the cell is a list to point at.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::EntryAction;
use crate::kit::EntryRow;
use crate::kit::NameTooltip;

const NOTE: &str = "One tooltip for a whole list, saying the whole of a name the row had to \
                    cut. Rest the pointer on a name that ends in an ellipsis, or Tab to an \
                    openable one: its whole path shows. A name that fits shows nothing. A path with no hyphens or spaces wraps inside the tooltip instead of scrolling. Move \
                    down the list while one is up and the next cut name shows at once.";

const NAMES: [&str; 6] = [
    "README.md",
    "raw/2026-10-15/plate-07/instrument-export-with-a-very-long-name.csv",
    "notes/kickoff-thread.md",
    "processed/normalised/plate-07-normalised-against-the-control-wells.parquet",
    "processed/summary.json",
    "raw/2026_10_15/plate_07/instrument_export/plate_07_raw_counts_before_normalisation_against_controls.csv",
];

#[component]
pub fn NameTooltipStories() -> impl IntoView {
    view! {
        <Story title="NameTooltip" note=NOTE>
            <Cell full=true label="a narrow list — hover the cut names, or Tab through them">
                <div style="width:360px">
                    <NameTooltip>
                        {NAMES
                            .into_iter()
                            .map(|name| {
                                view! {
                                    <EntryRow
                                        name=name
                                        size="4 KB"
                                        action=EntryAction::Open(Callback::new(|()| ()))
                                        plain=true
                                    />
                                }
                            })
                            .collect_view()}
                    </NameTooltip>
                </div>
            </Cell>
        </Story>
    }
}

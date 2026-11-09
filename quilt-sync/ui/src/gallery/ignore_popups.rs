//! The two popups a file row's `[⋯]` opens: *Ignore* and *Stop ignoring*,
//! each in the states it can reach, over fixtures. Open one, then switch the
//! theme: both follow it.

use leptos::prelude::*;

use crate::Scene;
use crate::commands::{ScopePreview, StopIgnoringPreview};
use crate::components::UnignorePopupData;
use crate::components::ignore_popup::{
    IgnoreDialog, IgnoreHint, Preview, StopIgnoringDialog, THIS_PACKAGE,
};
use quilt_ui_kit::{Button, Submit};

const PATH: &str = "raw/plate-07.csv";

fn unignore_data() -> UnignorePopupData {
    UnignorePopupData {
        namespace: "user/plate-07".to_string(),
        path: ".snakemake".to_string(),
        folder: true,
        pattern: ".snakemake/".to_string(),
        uri: None,
    }
}

fn counted() -> Preview {
    Preview::Ready(StopIgnoringPreview {
        this_package: ScopePreview {
            lines: vec!["!/.snakemake/".to_string()],
            files: 5412,
            bytes: 210_000_000,
        },
        all_packages: Some(ScopePreview {
            lines: vec!["!.snakemake/".to_string()],
            files: 5412,
            bytes: 210_000_000,
        }),
    })
}

/// A submit that is refused, so the refusal shows inside the popup.
fn refused(label: &'static str, words: &'static str) -> Submit {
    Submit::new(label, move || async move { Err(words.to_string()) })
}

#[component]
pub fn IgnorePopupsScene() -> impl IntoView {
    let ignoring = RwSignal::new(false);
    let pattern = RwSignal::new(PATH.to_string());
    // A fixture's answer: the suggested pattern matches, anything else does not.
    let hint = Signal::derive(move || {
        let typed = pattern.get();
        (!typed.trim().is_empty()).then(|| {
            if typed == PATH {
                IgnoreHint::WillBeIgnored(PATH.to_string())
            } else {
                IgnoreHint::NoMatch(PATH.to_string())
            }
        })
    });

    let stopping = RwSignal::new(false);
    let counting = RwSignal::new(false);
    let failed = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · the ignore popups"
            note="Ignore and Stop ignoring are popup forms: Return submits, Escape and \
                  Cancel close, and the backdrop closes nothing. A refusal shows inside \
                  the popup, above its fields; try either verb here to see one. Stop \
                  ignoring cannot be sent until its count is in."
        >
            <div class="g-inline">
                <Button on_click=move |_| ignoring.set(true)>"Ignore"</Button>
                <Button on_click=move |_| stopping.set(true)>"Stop ignoring"</Button>
                <Button on_click=move |_| counting.set(true)>"Stop ignoring, counting"</Button>
                <Button on_click=move |_| failed.set(true)>"Stop ignoring, count failed"</Button>
            </div>
            <IgnoreDialog
                open=ignoring
                pattern=pattern
                hint=hint
                submit=refused("Add to .quiltignore", "Could not ignore this file: no write access to .quiltignore")
                blocked=Signal::derive(move || pattern.with(|p| p.trim().is_empty()))
            />
            <StopIgnoringDialog
                open=stopping
                data=unignore_data()
                preview=Signal::stored(counted())
                scope=RwSignal::new(THIS_PACKAGE.to_string())
                lock=None
                submit=refused("Stop ignoring", "Could not stop ignoring this file: no write access to .quiltignore")
            />
            <StopIgnoringDialog
                open=counting
                data=unignore_data()
                preview=Signal::stored(Preview::Counting)
                scope=RwSignal::new(THIS_PACKAGE.to_string())
                lock=None
                submit=refused("Stop ignoring", "")
            />
            <StopIgnoringDialog
                open=failed
                data=unignore_data()
                preview=Signal::stored(Preview::Failed("Could not read the package's files.".to_string()))
                scope=RwSignal::new(THIS_PACKAGE.to_string())
                lock=None
                submit=refused("Stop ignoring", "")
            />
        </Scene>
    }
}

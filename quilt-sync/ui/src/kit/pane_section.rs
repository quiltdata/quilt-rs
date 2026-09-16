//! A labelled block inside a pane.
//!
//! # Why it is not a `Card`
//!
//! It carries a [`Card`](super::Card)'s label treatment and none of its surface.
//! The pane it sits in is already the box, and a bordered block inside a bordered
//! pane is the box-in-box the design rules out. So this is the half of `Card`
//! that is a heading, for the places where the container above already drew the
//! edge.
//!
//! # The nested level
//!
//! Resolve mode stacks one of these inside another — a section naming the choice,
//! holding one section per side. Two identical labels at two levels is a
//! hierarchy the reader cannot see, so `nested` both drops the heading a level
//! and takes the weight down with it.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/pane_section.module.scss");

#[component]
pub fn PaneSection(
    /// Optional, because a section can be the only thing in its pane and be
    /// named by the pane instead — resolve mode's outer block is titled by the
    /// mode it is in, and repeating the word above the sentence that explains it
    /// says it twice.
    #[prop(optional, into)]
    label: Option<String>,
    /// One level down, for a section inside a section.
    #[prop(optional)]
    nested: bool,
    children: Children,
) -> impl IntoView {
    let class = if nested {
        format!("{} {}", style::root, style::nested)
    } else {
        String::from(style::root)
    };

    view! {
        <section class=class>
            {label
                .map(|text| {
                    // `h4` under `h3`, never a skipped level: the pane is a `Card`
                    // and the page's regions are its `h2`s.
                    if nested {
                        view! { <h4 class=style::label>{text}</h4> }.into_any()
                    } else {
                        view! { <h3 class=style::label>{text}</h3> }.into_any()
                    }
                })}
            {children()}
        </section>
    }
}

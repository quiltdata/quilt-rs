//! Names and counts a run of rows.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/group_heading.module.scss");

#[component]
pub fn GroupHeading(
    #[prop(into)] title: String,
    /// How many rows follow. Always shown, including one — "1" is information, and
    /// hiding it would make a single-row group look like a header with a bug.
    count: usize,
    /// A shared cause affecting the whole group, such as
    /// `no access as analyst`. Only the bucket axis has one: a prefix spans
    /// buckets, so no cause is a property of the group.
    #[prop(optional, into)]
    annotation: Option<String>,
) -> impl IntoView {
    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_title = title.clone();
    view! {
        <div class=style::root>
            <span class=style::title title=full_title>{title}</span>
            {annotation
                .map(|note| {
                    let full = format!("— {note}");
                    let hint = full.clone();
                    view! {
                        <span class=style::annotation title=hint>{full}</span>
                    }
                })}
            <span class=style::count>{count}</span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `main_page.rs`'s pattern: mount a view into a fresh, attached `div` and
    /// hand back the element to query against.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// The bucket name and its cause annotation both ellipsise.
    #[wasm_bindgen_test]
    fn the_truncating_title_and_annotation_carry_their_whole_values() {
        let el = mount(|| {
            view! {
                <GroupHeading
                    title="a-very-long-bucket-name-for-a-research-programme"
                    count=3
                    annotation="no access as a-long-role-name"
                />
            }
        });
        let title = el
            .query_selector("[class*=title]")
            .unwrap()
            .expect("the title");
        assert_eq!(
            title.get_attribute("title").as_deref(),
            Some("a-very-long-bucket-name-for-a-research-programme"),
        );
        let note = el
            .query_selector("[class*=annotation]")
            .unwrap()
            .expect("the annotation");
        assert_eq!(
            note.get_attribute("title").as_deref(),
            Some("— no access as a-long-role-name"),
        );
    }
}

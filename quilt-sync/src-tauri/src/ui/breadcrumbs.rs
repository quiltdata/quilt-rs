use std::borrow::Cow;

use askama::Template;

use crate::routes::Paths;

#[derive(Template)]
#[template(path = "./components/breadcrumbs.html")]
pub struct TmplBreadcrumbs<'a> {
    pub list: Vec<BreadcrumbItem<'a>>,
}

pub struct Link<'a> {
    pub href: Paths,
    pub title: Cow<'a, str>,
}

impl<'a> Link<'a> {
    pub fn create<T: Into<Cow<'a, str>>>(href: Paths, title: T) -> BreadcrumbItem<'a> {
        BreadcrumbItem::Link(Link {
            href,
            title: title.into(),
        })
    }

    pub fn home() -> BreadcrumbItem<'a> {
        BreadcrumbItem::Link(Link {
            href: Paths::InstalledPackagesList,
            title: "".into(),
        })
    }
}

pub struct Current<'a> {
    pub title: Cow<'a, str>,
}

impl<'a> Current<'a> {
    pub fn create<T: Into<Cow<'a, str>>>(title: T) -> BreadcrumbItem<'a> {
        BreadcrumbItem::Current(Current {
            title: title.into(),
        })
    }
}

pub enum BreadcrumbItem<'a> {
    Current(Current<'a>),
    Link(Link<'a>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::Paths;

    #[test]
    fn test_breadcrumbs_with_current_item() {
        // Create breadcrumbs with a single current item
        let breadcrumbs = TmplBreadcrumbs {
            list: vec![Current::create("Current Page")],
        };

        // Render the HTML
        let html = breadcrumbs.to_string();

        // Check that the HTML contains the expected elements
        assert!(html.contains(r#"<nav class="qui-breadcrumbs">"#));
        assert!(html.contains(r#"<ul class="list">"#));
        assert!(html.contains(r#"<li class="item">"#));
        assert!(html.contains(
            r#"<strong class="qui-breadcrumb-current" title="Current Page">Current Page</strong>"#
        ));
    }

    #[test]
    fn test_breadcrumbs_with_link_item() {
        // Create breadcrumbs with a single link item
        let breadcrumbs = TmplBreadcrumbs {
            list: vec![Link::create(Paths::InstalledPackagesList, "Packages")],
        };

        // Render the HTML
        let html = breadcrumbs.to_string();

        // Check that the HTML contains the expected elements
        assert!(html.contains(r#"<nav class="qui-breadcrumbs">"#));
        assert!(html.contains(r#"<ul class="list">"#));
        assert!(html.contains(r#"<li class="item">"#));
        assert!(html.contains(r#"<a class="qui-breadcrumb-link""#));
        assert!(html.contains("Packages"));
    }

    #[test]
    fn test_breadcrumbs_with_multiple_items() {
        // Create breadcrumbs with multiple items (link and current)
        let breadcrumbs = TmplBreadcrumbs {
            list: vec![
                Link::create(Paths::InstalledPackagesList, "Home"),
                Current::create("Current Page"),
            ],
        };

        // Render the HTML
        let html = breadcrumbs.to_string();

        // Check that the HTML contains the expected elements and structure
        assert!(html.contains(r#"<nav class="qui-breadcrumbs">"#));
        assert!(html.contains(r#"<ul class="list">"#));
        assert!(html.contains(r#"<a class="qui-breadcrumb-link""#));
        assert!(html.contains("Home"));
        assert!(html.contains(
            r#"<strong class="qui-breadcrumb-current" title="Current Page">Current Page</strong>"#
        ));

        // Check that there are two list items
        let li_count = html.matches(r#"<li class="item">"#).count();
        assert_eq!(li_count, 2);
    }

    #[test]
    fn test_home_link() {
        // Test the home() helper method
        let home_item = Link::home();

        // Convert to breadcrumbs and render
        let breadcrumbs = TmplBreadcrumbs {
            list: vec![home_item],
        };

        let html = breadcrumbs.to_string();

        // The home link should have an empty title but valid href
        assert!(html.contains(r#"<a class="qui-breadcrumb-link""#));
        assert!(html.contains(r#"title="""#));
        assert!(html.contains(r#"href="/installed-packages-list""#));
    }

    #[test]
    fn test_empty_breadcrumbs() {
        // Create breadcrumbs with no items
        let breadcrumbs = TmplBreadcrumbs { list: vec![] };

        // Render the HTML
        let html = breadcrumbs.to_string();

        // Should still have the nav and ul elements, but no li elements
        assert!(html.contains(r#"<nav class="qui-breadcrumbs">"#));
        assert!(html.contains(r#"<ul class="list">"#));
        assert!(!html.contains(r#"<li class="item">"#));
    }
}

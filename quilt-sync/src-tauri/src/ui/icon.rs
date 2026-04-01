use askama::Template;

#[derive(Debug, Template)]
#[template(
    source = "<img class=\"qui-icon\" src=\"{{ self::get_src(self) }}\" />",
    ext = "txt"
)]
pub enum Icon {
    ArrowForward,
    Block,
    CloudDownload,
    CloudUpload,
    Commit,
    Done,
    FolderOpen,
    Gear,
    Merge,
    OpenInBrowser,
    OpenInNew,
    Refresh,
    Visibility,
    VisibilityOff,
    Warning,
}

fn get_src(icon: &Icon) -> &str {
    match icon {
        Icon::ArrowForward => "/assets/img/icons/arrow_forward.svg",
        Icon::Block => "/assets/img/icons/block.svg",
        Icon::CloudDownload => "/assets/img/icons/cloud_download.svg",
        Icon::CloudUpload => "/assets/img/icons/cloud_upload.svg",
        Icon::Commit => "/assets/img/icons/commit.svg",
        Icon::Done => "/assets/img/icons/done.svg",
        Icon::FolderOpen => "/assets/img/icons/folder_open.svg",
        Icon::Gear => "/assets/img/icons/gear.svg",
        Icon::Merge => "/assets/img/icons/merge.svg",
        Icon::OpenInBrowser => "/assets/img/icons/open_in_browser.svg",
        Icon::OpenInNew => "/assets/img/icons/open_in_new.svg",
        Icon::Refresh => "/assets/img/icons/refresh.svg",
        Icon::Visibility => "/assets/img/icons/visibility.svg",
        Icon::VisibilityOff => "/assets/img/icons/visibility_off.svg",
        Icon::Warning => "/assets/img/icons/warning.svg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_rendering() {
        // Test rendering of different icons
        let refresh_icon = Icon::Refresh;
        let done_icon = Icon::Done;

        // Check the rendered HTML output
        assert_eq!(
            refresh_icon.to_string(),
            r#"<img class="qui-icon" src="/assets/img/icons/refresh.svg" />"#
        );

        assert_eq!(
            done_icon.to_string(),
            r#"<img class="qui-icon" src="/assets/img/icons/done.svg" />"#
        );
    }
}

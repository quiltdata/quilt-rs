pub fn build_class(
    primary: bool,
    warning: bool,
    small: bool,
    large: bool,
    link: bool,
) -> String {
    let mut class = "qui-button".to_string();
    if primary {
        class.push_str(" primary");
    }
    if warning {
        class.push_str(" warning");
    }
    if small {
        class.push_str(" small");
    }
    if large {
        class.push_str(" large");
    }
    if link {
        class.push_str(" link");
    }
    class
}

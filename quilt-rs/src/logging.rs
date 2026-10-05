//! How a `QUILT_LOG` value becomes a log filter.
//!
//! Both front doors, the `quilt` CLI and the desktop app, read the same variable and
//! must apply the same rule, so the rule lives here. This module only turns a
//! value into `tracing` directives: it reads no environment variable and
//! installs no subscriber, so the library still only emits events.

/// The environment variable both front doors read their log filter from.
pub const LOG_ENV: &str = "QUILT_LOG";

/// Our crates' log targets, each named: `quilt` (the CLI binary) is not relied
/// on to match the others as a prefix.
pub const OUR_TARGETS: [&str; 4] = ["quilt", "quilt_rs", "quilt_uri", "quilt_sync"];

/// What every dependency logs at when a bare level is given.
const DEPENDENCY_LEVEL: &str = "warn";

/// The `tracing` directives for a `QUILT_LOG` (or `--log`) value, or `None`
/// when the value is empty, which counts as unset.
///
/// A bare level puts our crates at that level and every dependency at `warn`,
/// because the dependencies, not this code, are the volume. A level quieter
/// than `warn` (`error`, `off`) applies to everything instead, so dependencies
/// are never louder than asked. Case and surrounding spaces do not matter.
/// Anything else is directives, returned verbatim.
///
/// ```
/// use quilt_rs::logging::directives;
///
/// assert_eq!(
///     directives("debug").as_deref(),
///     Some("quilt=debug,quilt_rs=debug,quilt_uri=debug,quilt_sync=debug,warn"),
/// );
/// assert_eq!(directives("error").as_deref(), Some("error"));
/// assert_eq!(directives("quilt_rs=trace").as_deref(), Some("quilt_rs=trace"));
/// assert_eq!(directives(""), None);
/// ```
#[must_use]
pub fn directives(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let level = value.to_ascii_lowercase();
    match level.as_str() {
        "trace" | "debug" | "info" | "warn" => {
            let mut parts: Vec<String> = OUR_TARGETS
                .iter()
                .map(|target| format!("{target}={level}"))
                .collect();
            parts.push(DEPENDENCY_LEVEL.to_string());
            Some(parts.join(","))
        }
        "error" | "off" => Some(level),
        _ => Some(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours_at(level: &str) -> String {
        format!("quilt={level},quilt_rs={level},quilt_uri={level},quilt_sync={level},warn")
    }

    #[test]
    fn a_bare_level_puts_our_crates_at_it_and_dependencies_at_warn() {
        for level in ["trace", "debug", "info", "warn"] {
            assert_eq!(directives(level), Some(ours_at(level)), "{level}");
        }
    }

    #[test]
    fn a_level_quieter_than_warn_applies_to_everything() {
        assert_eq!(directives("error").as_deref(), Some("error"));
        assert_eq!(directives("off").as_deref(), Some("off"));
    }

    #[test]
    fn a_bare_level_ignores_case_and_surrounding_spaces() {
        assert_eq!(directives("DEBUG"), Some(ours_at("debug")));
        assert_eq!(directives(" Info \n"), Some(ours_at("info")));
        assert_eq!(directives(" OFF ").as_deref(), Some("off"));
    }

    #[test]
    fn directives_are_used_verbatim() {
        for value in [
            "quilt_rs=trace",
            "quilt_rs=trace,aws_smithy_runtime=debug",
            "quilt_sync=info,warn",
            // A target named like a level is still a directive.
            "debug,hyper=off",
        ] {
            assert_eq!(directives(value).as_deref(), Some(value), "{value}");
        }
    }

    #[test]
    fn an_empty_value_counts_as_unset() {
        assert_eq!(directives(""), None);
        assert_eq!(directives("   "), None);
    }
}

//! How a `QUILT_LOG` value becomes a log filter.
//!
//! Both front doors, the `quilt` CLI and the desktop app, read the same
//! variable and must apply the same rule, so the rule lives here. This module
//! only turns a value into `tracing` directives: it reads no environment
//! variable and installs no subscriber, so the library still only emits events.

/// The environment variable both front doors read their log filter from.
pub const LOG_ENV: &str = "QUILT_LOG";

/// Our crates' log targets, each named: `quilt` (the CLI binary) is not relied
/// on to match the others as a prefix.
pub const OUR_TARGETS: [&str; 4] = ["quilt", "quilt_rs", "quilt_uri", "quilt_sync"];

/// What every dependency logs at when a bare level is given.
const DEPENDENCY_LEVEL: &str = "warn";

const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];

/// A log filter value that is neither a level nor a list of directives.
///
/// Each app decides how to report it; the CLI refuses to run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not a log level or a list of directives")]
pub struct InvalidLogFilter {
    /// The value as given.
    pub value: String,
}

impl InvalidLogFilter {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

/// The `tracing` directives for a `QUILT_LOG` (or `--log`) value, or
/// `Ok(None)` when the value is empty, which counts as unset.
///
/// A bare level puts our crates at that level and every dependency at `warn`,
/// because the dependencies, not this code, are the volume. A level quieter
/// than `warn` (`error`, `off`) applies to everything instead, so dependencies
/// are never louder than asked. Case and surrounding spaces do not matter.
/// Anything else is directives, returned verbatim.
///
/// # Errors
///
/// A comma-separated part with no `=` that is not a level, such as `debgu` or
/// `quilt_rs`. `tracing` would read it as a target name and hide every other
/// line, so a typo in a level is an error rather than silence. This checks
/// the shape only: a caller that builds the filter still has to parse the
/// directives strictly, which catches `quilt_rs=lots`.
///
/// ```
/// use quilt_rs::logging::directives;
///
/// assert_eq!(
///     directives("debug").unwrap().as_deref(),
///     Some("quilt=debug,quilt_rs=debug,quilt_uri=debug,quilt_sync=debug,warn"),
/// );
/// assert_eq!(directives("error").unwrap().as_deref(), Some("error"));
/// assert_eq!(directives("quilt_rs=trace").unwrap().as_deref(), Some("quilt_rs=trace"));
/// assert_eq!(directives("").unwrap(), None);
/// assert!(directives("debgu").is_err());
/// ```
pub fn directives(value: &str) -> Result<Option<String>, InvalidLogFilter> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let level = trimmed.to_ascii_lowercase();
    match level.as_str() {
        "trace" | "debug" | "info" | "warn" => {
            let mut parts: Vec<String> = OUR_TARGETS
                .iter()
                .map(|target| format!("{target}={level}"))
                .collect();
            parts.push(DEPENDENCY_LEVEL.to_string());
            Ok(Some(parts.join(",")))
        }
        "error" | "off" => Ok(Some(level)),
        _ if trimmed.split(',').all(is_directive_shaped) => Ok(Some(trimmed.to_string())),
        _ => Err(InvalidLogFilter::new(value)),
    }
}

/// A part of a directive list: empty, `target=level`, or a bare level.
fn is_directive_shaped(part: &str) -> bool {
    let part = part.trim();
    part.is_empty() || part.contains('=') || LEVELS.contains(&part.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours_at(level: &str) -> Option<String> {
        Some(format!(
            "quilt={level},quilt_rs={level},quilt_uri={level},quilt_sync={level},warn"
        ))
    }

    fn ok(value: &str) -> Option<String> {
        directives(value).unwrap_or_else(|err| panic!("{value:?}: {err}"))
    }

    #[test]
    fn a_bare_level_puts_our_crates_at_it_and_dependencies_at_warn() {
        for level in ["trace", "debug", "info", "warn"] {
            assert_eq!(ok(level), ours_at(level), "{level}");
        }
    }

    #[test]
    fn a_level_quieter_than_warn_applies_to_everything() {
        assert_eq!(ok("error").as_deref(), Some("error"));
        assert_eq!(ok("off").as_deref(), Some("off"));
    }

    #[test]
    fn a_bare_level_ignores_case_and_surrounding_spaces() {
        assert_eq!(ok("DEBUG"), ours_at("debug"));
        assert_eq!(ok(" Info \n"), ours_at("info"));
        assert_eq!(ok(" OFF ").as_deref(), Some("off"));
    }

    #[test]
    fn directives_are_used_verbatim() {
        for value in [
            "quilt_rs=trace",
            "quilt_rs=trace,aws_smithy_runtime=debug",
            "quilt_sync=info,warn",
            "debug,hyper=off",
        ] {
            assert_eq!(ok(value).as_deref(), Some(value), "{value}");
        }
    }

    #[test]
    fn an_empty_value_counts_as_unset() {
        assert_eq!(ok(""), None);
        assert_eq!(ok("   "), None);
    }

    /// The case this guards: `debgu` used to read as a target name, which
    /// hid every quilt line, warnings and errors included.
    #[test]
    fn a_word_that_is_not_a_level_is_an_error() {
        for value in [
            "debgu",
            "quilt_rs",
            "debgu,hyper=off",
            "quilt_rs=trace,hyper",
        ] {
            assert_eq!(
                directives(value),
                Err(InvalidLogFilter::new(value)),
                "{value}"
            );
        }
    }

    #[test]
    fn the_error_names_the_value_as_given() {
        assert_eq!(
            InvalidLogFilter::new(" debgu").to_string(),
            "\" debgu\" is not a log level or a list of directives"
        );
    }
}

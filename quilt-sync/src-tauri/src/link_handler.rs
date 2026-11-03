//! Whether Windows opens `quilt+s3` and `quilt` links with this copy of the app.
//!
//! Registering the schemes at startup rewrites their classes, but Windows can still
//! send a link elsewhere: a per-user `UserChoice` outranks the class, and an app
//! cannot rewrite it. So the app asks Windows which program it will actually run.

use serde::Serialize;

/// The schemes in the `deep-link` list in `tauri.conf.json`.
#[cfg(any(windows, test))]
const SCHEMES: [&str; 2] = ["quilt+s3", "quilt"];

/// A link type that Windows does not open with this copy of the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignHandler {
    pub scheme: String,
    /// The program Windows opens these links with; `None` when there is none.
    pub program: Option<String>,
}

/// What the startup check found. Empty when every link type opens this copy,
/// and always empty off Windows.
#[derive(Default)]
pub struct LinkHandlers(pub Vec<ForeignHandler>);

/// Ask Windows about each scheme, and report a mismatch once.
///
/// The paths go to the log file only: they carry the user's name, and `debug`
/// never reaches the crash reporter.
#[cfg(windows)]
pub fn check(telemetry: &crate::telemetry::Telemetry) -> LinkHandlers {
    use crate::telemetry::prelude::*;

    let ours = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(err) => {
            warn!("Cannot find QuiltSync's own program to check its links: {err}");
            return LinkHandlers::default();
        }
    };
    let foreign = foreign_handlers(&ours, shell::handler_of);
    for handler in &foreign {
        debug!(
            "Windows opens {} links with {:?}, not {ours}",
            handler.scheme, handler.program
        );
    }
    if !foreign.is_empty() {
        telemetry.report_anomaly("Windows opens Quilt links with another program");
    }
    LinkHandlers(foreign)
}

/// The schemes whose handler, as `handler_of` names it, is not `ours`.
#[cfg(any(windows, test))]
fn foreign_handlers(
    ours: &str,
    handler_of: impl Fn(&str) -> Option<String>,
) -> Vec<ForeignHandler> {
    let ours = normalise(ours);
    SCHEMES
        .iter()
        .filter_map(|scheme| {
            let program = handler_of(scheme);
            match &program {
                Some(path) if normalise(path) == ours => None,
                _ => Some(ForeignHandler {
                    scheme: (*scheme).to_owned(),
                    program,
                }),
            }
        })
        .collect()
}

/// A Windows path in a form two names for one file share: canonical when the
/// file exists, without the `\\?\` prefix that canonical form adds, with
/// backslashes, and in one case, since Windows paths ignore case.
#[cfg(any(windows, test))]
fn normalise(path: &str) -> String {
    let path = path.trim_matches('"');
    let canonical = std::fs::canonicalize(path)
        .map_or_else(|_| path.to_owned(), |p| p.to_string_lossy().into_owned());
    canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical)
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(windows)]
mod shell {
    use windows_sys::Win32::UI::Shell::ASSOCF_IS_PROTOCOL;
    use windows_sys::Win32::UI::Shell::ASSOCSTR_EXECUTABLE;
    use windows_sys::Win32::UI::Shell::AssocQueryStringW;

    /// The longest path Windows allows, in UTF-16 units.
    const MAX_PATH_UNITS: u32 = 32_768;

    /// The program Windows runs for `scheme` links, `UserChoice` included.
    pub fn handler_of(scheme: &str) -> Option<String> {
        let scheme: Vec<u16> = scheme.encode_utf16().chain(Some(0)).collect();
        let mut out = vec![0u16; MAX_PATH_UNITS as usize];
        let mut len = MAX_PATH_UNITS;
        // SAFETY: `scheme` ends in a NUL, and `len` is `out`'s length.
        let result = unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_EXECUTABLE,
                scheme.as_ptr(),
                std::ptr::null(),
                out.as_mut_ptr(),
                &raw mut len,
            )
        };
        if result != 0 {
            return None;
        }
        // `len` counts the closing NUL.
        let path = String::from_utf16_lossy(&out[..(len as usize).saturating_sub(1)]);
        (!path.is_empty()).then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = r"C:\Users\a-user\AppData\Local\QuiltSync\quilt-sync.exe";

    fn all_open_with(path: &'static str) -> impl Fn(&str) -> Option<String> {
        move |_| Some(path.to_owned())
    }

    #[test]
    fn the_same_program_in_another_spelling_is_ours() {
        for spelling in [
            OURS,
            r"c:\users\A-USER\appdata\local\quiltsync\QUILT-SYNC.EXE",
            r"\\?\C:\Users\a-user\AppData\Local\QuiltSync\quilt-sync.exe",
            "C:/Users/a-user/AppData/Local/QuiltSync/quilt-sync.exe",
            r#""C:\Users\a-user\AppData\Local\QuiltSync\quilt-sync.exe""#,
        ] {
            assert_eq!(
                foreign_handlers(OURS, |_| Some(spelling.to_owned())),
                Vec::new(),
                "{spelling}"
            );
        }
    }

    #[test]
    fn another_program_is_named_for_each_scheme() {
        let other = r"C:\Users\a-user\Downloads\quilt-sync.exe";
        assert_eq!(
            foreign_handlers(OURS, all_open_with(other)),
            vec![
                ForeignHandler {
                    scheme: "quilt+s3".to_owned(),
                    program: Some(other.to_owned()),
                },
                ForeignHandler {
                    scheme: "quilt".to_owned(),
                    program: Some(other.to_owned()),
                },
            ]
        );
    }

    #[test]
    fn a_scheme_with_no_program_is_foreign() {
        let found = foreign_handlers(OURS, |scheme| (scheme == "quilt").then(|| OURS.to_owned()));
        assert_eq!(
            found,
            vec![ForeignHandler {
                scheme: "quilt+s3".to_owned(),
                program: None,
            }]
        );
    }
}

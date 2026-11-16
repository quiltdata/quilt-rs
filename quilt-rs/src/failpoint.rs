//! Named points where a crash test kills the process.
//!
//! Compiled in only for tests and the non-default `failpoints` feature;
//! elsewhere [`hit`] does nothing. A process whose `QUILT_FAILPOINT` names the
//! point it reaches writes `<name>.reached` into `QUILT_FAILPOINT_DIR` and
//! parks that thread for good, so a parent that sees the file can kill it
//! there, with nothing unwinding.

use std::fmt::Display;

#[cfg(any(test, feature = "failpoints"))]
pub(crate) const NAME_VAR: &str = "QUILT_FAILPOINT";

#[cfg(any(test, feature = "failpoints"))]
pub(crate) const DIR_VAR: &str = "QUILT_FAILPOINT_DIR";

/// Stops here for good when `QUILT_FAILPOINT` is `name`.
#[cfg(any(test, feature = "failpoints"))]
pub(crate) fn hit(name: impl Display) {
    let Some(wanted) = std::env::var_os(NAME_VAR) else {
        return;
    };
    let name = name.to_string();
    if wanted != name.as_str() {
        return;
    }
    if let Some(dir) = std::env::var_os(DIR_VAR) {
        let _ = std::fs::write(
            std::path::Path::new(&dir).join(format!("{name}.reached")),
            b"",
        );
    }
    loop {
        std::thread::park();
    }
}

#[cfg(not(any(test, feature = "failpoints")))]
pub(crate) fn hit(_: impl Display) {}

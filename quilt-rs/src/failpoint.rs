//! Named points where a crash test kills the process.
//!
//! Compiled in only for tests and the non-default `failpoints` feature;
//! elsewhere [`hit`] does nothing. A process whose `QUILT_FAILPOINT` names the
//! point it reaches writes `<name>.reached` into `QUILT_FAILPOINT_DIR` and
//! parks that thread, so a parent that sees the file can kill it there, with
//! nothing unwinding, or let it go on by creating `<name>.resume` beside it.

use std::fmt::Display;

#[cfg(any(test, feature = "failpoints"))]
pub(crate) const NAME_VAR: &str = "QUILT_FAILPOINT";

#[cfg(any(test, feature = "failpoints"))]
pub(crate) const DIR_VAR: &str = "QUILT_FAILPOINT_DIR";

/// Stops here when `QUILT_FAILPOINT` is `name`, until told to go on.
#[cfg(any(test, feature = "failpoints"))]
pub(crate) fn hit(name: impl Display) {
    let Some(wanted) = std::env::var_os(NAME_VAR) else {
        return;
    };
    let name = name.to_string();
    if wanted != name.as_str() {
        return;
    }
    let Some(dir) = std::env::var_os(DIR_VAR) else {
        loop {
            std::thread::park();
        }
    };
    let dir = std::path::Path::new(&dir);
    let _ = std::fs::write(dir.join(format!("{name}.reached")), b"");
    let resume = dir.join(format!("{name}.resume"));
    while !resume.exists() {
        std::thread::park_timeout(std::time::Duration::from_millis(5));
    }
}

#[cfg(not(any(test, feature = "failpoints")))]
pub(crate) fn hit(_: impl Display) {}

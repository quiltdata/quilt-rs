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

#[cfg(test)]
tokio::task_local! {
    /// The point a test stops at, and how many passes of it to let by first.
    static STOP: (&'static str, std::cell::Cell<usize>);
}

/// Fails where a test running under [`stopping_at`] names `name`, so the
/// operation is left as a crash there would leave it, in-process and on
/// every OS. Elsewhere it does nothing.
#[cfg_attr(
    not(test),
    allow(clippy::unnecessary_wraps, reason = "it fails only under test")
)]
pub(crate) fn stop(name: &str) -> crate::Res {
    #[cfg(test)]
    if STOP
        .try_with(|(at, passes)| *at == name && passes.replace(passes.get().wrapping_sub(1)) == 0)
        .unwrap_or(false)
    {
        return Err(std::io::Error::other(format!("stopped at {name}")).into());
    }
    #[cfg(not(test))]
    let _ = name;
    Ok(())
}

/// Runs `operation`, stopping it at `name` once `passes` passes of it have
/// gone by; see [`stop`].
#[cfg(test)]
pub(crate) async fn stopping_at<F: std::future::Future>(
    name: &'static str,
    passes: usize,
    operation: F,
) -> F::Output {
    STOP.scope((name, std::cell::Cell::new(passes)), operation)
        .await
}

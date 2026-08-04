use semver::Version;

use crate::env;
use crate::telemetry::{AmbientHost, Sinks};

fn get_sentry_dsn() -> Option<sentry::types::Dsn> {
    env::sentry_dsn().and_then(|dsn_str| {
        dsn_str.parse().ok().or_else(|| {
            eprintln!("Warning: Invalid SENTRY_DSN format: {dsn_str}");
            None
        })
    })
}

/// Tag every outgoing event with the deployment in play, read from `host`.
///
/// The hook is the only mechanism that holds for a crash: it runs on the single
/// process-wide client as the event leaves, so it covers a panic, a captured
/// error and a user-filed crash report alike, on any thread. Tagging the scope
/// instead would reach only the thread that set the tag — see [`AmbientHost`].
fn tag_host(host: AmbientHost) -> sentry::ClientOptions {
    let before_send = move |mut event: sentry::protocol::Event<'static>| {
        if let Some(host) = host.lock().ok().and_then(|host| host.clone()) {
            event
                .tags
                .insert("quilt_host".to_string(), host.to_string());
        }
        Some(event)
    };

    sentry::ClientOptions::new().before_send(before_send)
}

pub fn sentry_config(
    version: &Version,
    sinks: Sinks,
    host: AmbientHost,
) -> Option<sentry::ClientOptions> {
    let dsn = get_sentry_dsn();
    if dsn.is_none() {
        eprintln!("No SENTRY_DSN configured, Sentry disabled");
    }
    dsn.map(|dsn| {
        // `ClientOptions` is `#[non_exhaustive]` as of sentry 0.49, so it is built
        // through the setters. `dsn` is assigned directly because the `dsn` setter
        // takes a `&str` and panics on a malformed value — `get_sentry_dsn` already
        // parsed it and warns instead.
        let mut options = tag_host(host).release(version.to_string());
        // Without this, a local build's crashes are indistinguishable from a
        // user's. The `None` arm is unreachable here — a disabled build never
        // reaches this function — but it is the compiler's business to know that,
        // not this call site's to assume it.
        if let Some(environment) = sinks.environment() {
            options = options.environment(environment);
        }
        options.dsn = Some(dsn);
        options
    })
}

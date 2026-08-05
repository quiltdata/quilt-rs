use semver::Version;

use crate::env;
use crate::telemetry::{AmbientHost, InstallId};

fn get_sentry_dsn() -> Option<sentry::types::Dsn> {
    env::sentry_dsn().and_then(|dsn_str| {
        dsn_str.parse().ok().or_else(|| {
            eprintln!("Warning: Invalid SENTRY_DSN format: {dsn_str}");
            None
        })
    })
}

/// Stamp every outgoing event with the deployment in play (read from `host`) and
/// this install's identity.
///
/// The hook is the only mechanism that holds for a crash: it runs on the single
/// process-wide client as the event leaves, so it covers a panic, a captured
/// error and a user-filed crash report alike, on any thread. Setting the scope
/// instead would reach only the thread that set it — see [`AmbientHost`].
///
/// The identity rides here for the same reason rather than because it varies: it
/// is fixed for the process, but a scope carrying it would still reach only the
/// threads that snapshotted after it was set. One mechanism, both facts.
fn stamp_event(install_id: Option<InstallId>, host: AmbientHost) -> sentry::ClientOptions {
    // Built once, not per event: the identity is fixed for the process, so the
    // hook clones a finished value rather than reassembling it on every send.
    // `user.id` and nothing else — the install identity is all we know, and it is
    // deliberately not a person.
    let user = install_id.map(|install_id| sentry::User {
        id: Some(install_id.as_str().to_owned()),
        ..Default::default()
    });

    let before_send = move |mut event: sentry::protocol::Event<'static>| {
        if let Some(host) = host.lock().ok().and_then(|host| host.clone()) {
            event
                .tags
                .insert("quilt_host".to_string(), host.to_string());
        }
        event.user.clone_from(&user);
        Some(event)
    };

    sentry::ClientOptions::new().before_send(before_send)
}

pub fn sentry_config(
    version: &Version,
    install_id: Option<InstallId>,
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
        //
        // A constant `environment`, because only a release build ever gets here —
        // see [`Sinks`](crate::telemetry::Sinks). Separating *kinds* of release
        // (an internal build from a customer's) is a distinct question and wants
        // more than two values, so it belongs to whoever takes that on.
        let mut options = stamp_event(install_id, host)
            .release(version.to_string())
            .environment("production");
        options.dsn = Some(dsn);
        options
    })
}

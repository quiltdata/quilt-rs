use semver::Version;

use crate::env;

fn get_sentry_dsn() -> Option<sentry::types::Dsn> {
    env::sentry_dsn().and_then(|dsn_str| {
        dsn_str.parse().ok().or_else(|| {
            eprintln!("Warning: Invalid SENTRY_DSN format: {}", dsn_str);
            None
        })
    })
}

pub fn sentry_config(version: &Version) -> Option<sentry::ClientOptions> {
    let dsn = get_sentry_dsn();
    if dsn.is_none() {
        eprintln!("No SENTRY_DSN configured, Sentry disabled");
    }
    dsn.map(|dsn| sentry::ClientOptions {
        dsn: Some(dsn),
        release: Some(version.to_string().into()),
        ..Default::default()
    })
}

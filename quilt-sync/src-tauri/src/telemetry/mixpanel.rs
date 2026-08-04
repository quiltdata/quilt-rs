//! Sending events to Mixpanel. *What* we report lives in
//! [`event`](super::event).

use std::collections::HashMap;
use std::sync::Arc;

use mixpanel_rs::{Config, Mixpanel};
use serde_json::Value;

use crate::env;
use crate::error::TelemetryError;
use crate::telemetry::{MixpanelEvent, Sinks};

/// The event as Mixpanel receives it: its name, and its properties.
///
/// Both come out of the adjacently tagged serialization — `{"t": "event_name"}`
/// for an event with no payload, `{"t": "event_name", "c": {…}}` for one with a
/// payload. Nothing is merged in here: an event that concerns a deployment
/// carries the host in its own payload, so serde has already put it in `c`.
pub fn event_payload(
    event: &MixpanelEvent,
) -> crate::Result<(String, Option<HashMap<String, Value>>)> {
    let Value::Object(map) = serde_json::to_value(event)? else {
        // Unreachable with adjacently tagged serialization.
        return Err(TelemetryError::Serialize(
            "Expected object from adjacently tagged serialization".to_string(),
        )
        .into());
    };

    let Some(Value::String(event_name)) = map.get("t") else {
        return Err(TelemetryError::Serialize("Failed to serialize event name".to_string()).into());
    };

    let properties = map
        .get("c")
        .and_then(|v| v.as_object())
        .map(|obj| obj.clone().into_iter().collect());

    Ok((event_name.clone(), properties))
}

/// Where an event goes.
///
/// A local build resolves to [`Self::DryRun`] and **never constructs a client**:
/// there are no credentials to find and nothing that could reach a real project
/// by accident. That is the isolation — structural, not a flag on a request whose
/// server-side meaning we would have to trust.
#[derive(Clone)]
pub enum Analytics {
    /// A configured sink: the event is sent.
    Live(Arc<Mixpanel>),
    /// A local build: the event is rendered to its wire form and written to the
    /// developer's console. Nothing leaves the machine.
    DryRun,
    /// No sink: the event is dropped.
    Off,
}

impl Analytics {
    pub fn resolve(sinks: Sinks) -> Self {
        match sinks {
            Sinks::Disabled => Self::Off,
            Sinks::Development => Self::DryRun,
            Sinks::Production => mixpanel_config()
                .map_or(Self::Off, |(token, config)| {
                    Self::Live(Arc::new(Mixpanel::init(&token, Some(config))))
                }),
        }
    }
}

fn mixpanel_config() -> Option<(String, Config)> {
    let token = env::mixpanel_project_token();
    let secret = env::mixpanel_api_secret();
    if token.is_none() {
        eprintln!("No MIXPANEL_PROJECT_TOKEN configured, Mixpanel disabled");
    }
    if secret.is_none() {
        eprintln!("No MIXPANEL_API_SECRET configured");
    }
    token.map(|token| {
        let config = Config {
            secret,
            ..Default::default()
        };
        (token, config)
    })
}

/// The event as a dry run reports it — the **same** name and properties
/// [`event_payload`] hands the live path, so a developer reads the payload
/// rather than a paraphrase of it that could drift from it.
fn dry_run_line(event: &MixpanelEvent) -> crate::Result<String> {
    let (name, properties) = event_payload(event)?;
    Ok(match properties {
        Some(properties) => {
            // `properties` came from `serde_json::to_value`, so re-serializing it
            // cannot fail; the fallback keeps the event visible rather than
            // turning a formatting slip into a lost observation.
            let rendered = serde_json::to_string(&properties)
                .unwrap_or_else(|_| "<unrenderable>".to_string());
            format!("telemetry(dry-run) {name} {rendered}")
        }
        None => format!("telemetry(dry-run) {name}"),
    })
}

pub async fn track_event(analytics: &Analytics, event: &MixpanelEvent) -> crate::Result<()> {
    match analytics {
        Analytics::Live(mixpanel) => {
            let (event_name, properties) = event_payload(event)?;
            mixpanel.track(&event_name, properties).await?;
        }
        // Deliberately `eprintln!` and not `info!`: this is console output for
        // whoever is running the app, not application logging, and routing it
        // through the subscriber would subject it to the very filter default a
        // developer is often here to observe. It also matches how this module
        // already reports its configuration.
        Analytics::DryRun => eprintln!("{}", dry_run_line(event)?),
        Analytics::Off => {}
    }
    Ok(())
}

pub fn init(analytics: &Analytics) {
    if matches!(analytics, Analytics::Off) {
        return;
    }
    let analytics = analytics.clone();
    tauri::async_runtime::spawn(async move {
        // App launch precedes any host, so it carries no payload at all.
        if let Err(err) = track_event(&analytics, &MixpanelEvent::AppLaunched).await {
            eprintln!("Failed to track app launch: {err}");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use quilt_uri::Host;

    use super::*;
    use crate::Result;
    use crate::telemetry::event::{
        AuthEvent, LoginEvent, LoginFlow, PackageEvent, PackageFileEvent, RemotePackageEvent,
    };

    fn host() -> Host {
        Host::from_str("example.quilt.dev").expect("valid test host")
    }

    fn host_value() -> Value {
        Value::String("example.quilt.dev".to_string())
    }

    /// The events that carry no payload: name only, no properties object.
    #[test]
    fn test_events_without_a_payload() -> Result {
        for (event, expected) in [
            (MixpanelEvent::AppLaunched, "app_launched"),
            (MixpanelEvent::SetupCompleted, "setup_completed"),
            (
                MixpanelEvent::DirectoryPickerOpened,
                "directory_picker_opened",
            ),
            (MixpanelEvent::HomeDirOpened, "home_dir_opened"),
            (MixpanelEvent::WebBrowserOpened, "web_browser_opened"),
            (MixpanelEvent::DebugDotQuiltOpened, "debug_dot_quilt_opened"),
            (MixpanelEvent::DebugLogsOpened, "debug_logs_opened"),
            (MixpanelEvent::DataDirOpened, "data_dir_opened"),
            (MixpanelEvent::DiagnosticLogsSaved, "diagnostic_logs_saved"),
            (MixpanelEvent::CrashReportSent, "crash_report_sent"),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert!(props.is_none(), "{expected} must carry no properties");
        }

        Ok(())
    }

    /// A remote operation reports the deployment it ran against.
    #[test]
    fn test_remote_package_event_reports_its_host() -> Result {
        let event = MixpanelEvent::PackagePulled(RemotePackageEvent::for_host(Some(host())));

        let (name, props) = event_payload(&event)?;

        assert_eq!(name, "package_pulled");
        let props = props.expect("host");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(props.len(), 1);

        Ok(())
    }

    /// Every operation whose host the caller supplies must still emit when the
    /// caller supplied none — an unattributed event, never a dropped one.
    #[test]
    fn test_caller_supplied_events_emit_without_a_host() -> Result {
        for (event, expected) in [
            (
                MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(None)),
                "package_pushed",
            ),
            (
                MixpanelEvent::PackageCommitted(PackageEvent::for_uri(None)),
                "package_committed",
            ),
            (
                MixpanelEvent::PackageCreated(PackageEvent::hostless()),
                "package_created",
            ),
            (
                MixpanelEvent::FileRevealed(PackageFileEvent::for_uri(None)),
                "file_revealed",
            ),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert_eq!(
                props.expect("payload").get("host"),
                Some(&Value::Null),
                "{expected} must report a null host, not omit the property"
            );
        }

        Ok(())
    }

    /// The wire form a login event sent when `host` was a field of the event
    /// itself: `host` beside `flow`, same names, same string value.
    #[test]
    fn test_user_logged_in_keeps_its_wire_shape() -> Result {
        let event = MixpanelEvent::UserLoggedIn(LoginEvent {
            host: host(),
            flow: LoginFlow::OAuth,
        });

        let (name, props) = event_payload(&event)?;

        assert_eq!(name, "user_logged_in");
        let props = props.expect("host and flow");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(props.get("flow"), Some(&Value::String("oauth".to_string())));

        Ok(())
    }

    #[test]
    fn test_auth_events_report_a_guaranteed_host() -> Result {
        for (event, expected) in [
            (
                MixpanelEvent::RoleSwitched(AuthEvent { host: host() }),
                "role_switched",
            ),
            // `snake_case` splits the leading acronym, so this event has always
            // been reported under this slightly odd name. Renaming it would break
            // the continuity of its history, so the name stays and this records it.
            (
                MixpanelEvent::OAuthLoginInitiated(AuthEvent { host: host() }),
                "o_auth_login_initiated",
            ),
            (
                MixpanelEvent::AuthErased(AuthEvent { host: host() }),
                "auth_erased",
            ),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert_eq!(props.expect("host").get("host"), Some(&host_value()));
        }

        Ok(())
    }

    /// The accessor the crash-report cell reads. Exhaustive by construction —
    /// the compiler rejects a new variant that does not answer this.
    /// The dry-run and off arms are reachable and never fail the caller — a
    /// telemetry sink must not be able to break the command it rides on. This
    /// exercises the arms; that the line reaches a terminal is not something a
    /// test can see.
    #[tokio::test]
    async fn the_silent_arms_never_fail_the_caller() -> Result {
        track_event(&Analytics::DryRun, &MixpanelEvent::AppLaunched).await?;
        track_event(&Analytics::Off, &MixpanelEvent::AppLaunched).await?;
        Ok(())
    }

    /// A dry run is only useful if what it prints is what would have been sent.
    /// These assert the line carries the wire *name* and the wire *properties* —
    /// so a developer reading the console is reading the payload, and a payload
    /// bug is visible there rather than hidden behind a friendlier rendering.
    #[test]
    fn dry_run_reports_the_wire_name_and_properties() -> Result {
        let line = dry_run_line(&MixpanelEvent::UserLoggedIn(LoginEvent {
            host: host(),
            flow: LoginFlow::OAuth,
        }))?;

        assert!(line.contains("user_logged_in"), "wire name missing: {line}");
        assert!(
            line.contains("example.quilt.dev"),
            "host property missing: {line}"
        );
        assert!(line.contains("oauth"), "flow property missing: {line}");

        Ok(())
    }

    /// An event with no payload prints no properties object — the same
    /// distinction the wire form makes, kept visible rather than smoothed into an
    /// empty `{}`.
    #[test]
    fn dry_run_omits_properties_when_the_event_has_none() -> Result {
        let line = dry_run_line(&MixpanelEvent::AppLaunched)?;

        assert!(line.ends_with("app_launched"), "unexpected trailer: {line}");

        Ok(())
    }

    /// An unattributed event still prints, with an explicit null — the emitter
    /// failed to supply context and that must be readable, not invisible.
    #[test]
    fn dry_run_shows_an_absent_host_as_null() -> Result {
        let line = dry_run_line(&MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(
            None,
        )))?;

        assert!(line.contains("package_pushed"), "wire name missing: {line}");
        assert!(line.contains("null"), "absent host not visible: {line}");

        Ok(())
    }

    #[test]
    fn test_host_accessor_matches_the_payload() {
        assert_eq!(
            MixpanelEvent::PackagePulled(RemotePackageEvent::for_host(Some(host()))).host(),
            Some(&host())
        );
        assert_eq!(
            MixpanelEvent::AuthErased(AuthEvent { host: host() }).host(),
            Some(&host())
        );
        assert_eq!(MixpanelEvent::AppLaunched.host(), None);
        assert_eq!(
            MixpanelEvent::PackageCommitted(PackageEvent::for_uri(None)).host(),
            None
        );
    }
}

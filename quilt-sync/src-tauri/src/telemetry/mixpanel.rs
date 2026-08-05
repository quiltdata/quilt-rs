//! Sending events to Mixpanel. *What* we report lives in
//! [`event`](super::event).

use std::collections::HashMap;
use std::sync::Arc;

use mixpanel_rs::{Config, Mixpanel};
use serde_json::Value;

use crate::env;
use crate::error::TelemetryError;
use crate::telemetry::{InstallId, MixpanelEvent, Sinks};

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
            Sinks::Development => Self::DryRun,
            Sinks::Production => mixpanel_config().map_or(Self::Off, |(token, config)| {
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

/// The event as the ingest API receives it: what the event says, plus this
/// install's identity.
///
/// Kept separate from [`event_payload`] so the two stay honest about whose fact
/// each property is. The event's own payload is a property of the *event*, pinned
/// by its own tests; `distinct_id` is a property of the *install* and belongs to
/// nothing the vocabulary describes. An event with no payload of its own still
/// gets a properties object here, because it still has an identity.
fn wire_payload(
    event: &MixpanelEvent,
    install_id: Option<&InstallId>,
) -> crate::Result<(String, Option<HashMap<String, Value>>)> {
    let (name, properties) = event_payload(event)?;

    let Some(install_id) = install_id else {
        return Ok((name, properties));
    };

    let mut properties = properties.unwrap_or_default();
    properties.insert(
        "distinct_id".to_string(),
        Value::String(install_id.as_str().to_owned()),
    );
    Ok((name, Some(properties)))
}

/// The event as a dry run reports it — the **same** name and properties the live
/// path sends, identity included, so a developer reads the payload rather than a
/// paraphrase of it that could drift from it.
fn dry_run_line(event: &MixpanelEvent, install_id: Option<&InstallId>) -> crate::Result<String> {
    let (name, properties) = wire_payload(event, install_id)?;
    Ok(match properties {
        Some(properties) => {
            // `properties` came from `serde_json::to_value`, so re-serializing it
            // cannot fail; the fallback keeps the event visible rather than
            // turning a formatting slip into a lost observation.
            let rendered =
                serde_json::to_string(&properties).unwrap_or_else(|_| "<unrenderable>".to_string());
            format!("telemetry(dry-run) {name} {rendered}")
        }
        None => format!("telemetry(dry-run) {name}"),
    })
}

pub async fn track_event(
    analytics: &Analytics,
    event: &MixpanelEvent,
    install_id: Option<&InstallId>,
) -> crate::Result<()> {
    match analytics {
        Analytics::Live(mixpanel) => {
            let (event_name, properties) = wire_payload(event, install_id)?;
            mixpanel.track(&event_name, properties).await?;
        }
        // Deliberately `eprintln!` and not `info!`: this is console output for
        // whoever is running the app, not application logging, and routing it
        // through the subscriber would subject it to the very filter default a
        // developer is often here to observe. It also matches how this module
        // already reports its configuration.
        Analytics::DryRun => eprintln!("{}", dry_run_line(event, install_id)?),
        Analytics::Off => {}
    }
    Ok(())
}

pub fn init(analytics: &Analytics, install_id: Option<InstallId>) {
    if matches!(analytics, Analytics::Off) {
        return;
    }
    let analytics = analytics.clone();
    tauri::async_runtime::spawn(async move {
        // App launch precedes any host, so it carries none — but it does carry an
        // identity, which is what makes a launch countable as a person's.
        if let Err(err) =
            track_event(&analytics, &MixpanelEvent::AppLaunched, install_id.as_ref()).await
        {
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
    fn install_id() -> InstallId {
        // Constructed through `load` so the test cannot drift from the real
        // shape: nothing else may mint an identity.
        let dir = tempfile::TempDir::new().expect("tempdir");
        InstallId::load(dir.path()).expect("an id")
    }

    /// The identity rides on the wire, and it rides on an event that has no
    /// payload of its own — which is the case that matters most, since app launch
    /// is the head of every funnel and is exactly what could not be counted
    /// before.
    #[test]
    fn the_identity_reaches_a_payloadless_event() -> Result {
        let id = install_id();

        let (name, props) = wire_payload(&MixpanelEvent::AppLaunched, Some(&id))?;

        assert_eq!(name, "app_launched");
        let props = props.expect("an identity, even with no payload of its own");
        assert_eq!(
            props.get("distinct_id"),
            Some(&Value::String(id.as_str().to_owned()))
        );
        assert_eq!(props.len(), 1, "nothing else is invented: {props:?}");

        Ok(())
    }

    /// The identity is added beside the event's own properties, never instead of
    /// them — the host rule and the identity are independent facts.
    #[test]
    fn the_identity_does_not_displace_the_event_payload() -> Result {
        let id = install_id();

        let (name, props) = wire_payload(
            &MixpanelEvent::UserLoggedIn(LoginEvent {
                host: host(),
                flow: LoginFlow::OAuth,
            }),
            Some(&id),
        )?;

        assert_eq!(name, "user_logged_in");
        let props = props.expect("payload and identity");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(props.get("flow"), Some(&Value::String("oauth".to_string())));
        assert_eq!(
            props.get("distinct_id"),
            Some(&Value::String(id.as_str().to_owned()))
        );

        Ok(())
    }

    /// With no identity the event still goes, unchanged — an install that could
    /// not persist one is unattributed, never silent. Anything else would lose the
    /// events of exactly the machines having trouble.
    #[test]
    fn an_event_without_an_identity_is_unchanged() -> Result {
        assert_eq!(
            wire_payload(&MixpanelEvent::AppLaunched, None)?,
            event_payload(&MixpanelEvent::AppLaunched)?,
        );
        assert_eq!(
            wire_payload(
                &MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(None)),
                None
            )?,
            event_payload(&MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(
                None
            )))?,
        );

        Ok(())
    }

    /// The dry run shows the identity, which is how it gets verified locally at
    /// all — the rig would be blind to the one property added here otherwise.
    #[test]
    fn the_dry_run_shows_the_identity() -> Result {
        let id = install_id();

        let line = dry_run_line(&MixpanelEvent::AppLaunched, Some(&id))?;

        assert!(line.contains("app_launched"), "wire name missing: {line}");
        assert!(line.contains(id.as_str()), "identity missing: {line}");

        Ok(())
    }

    /// Autosync gets its **own** names rather than folding into the manual
    /// series. Pinned here because that was a deliberate call: adding background
    /// work to `package_published` would silently redefine a series already being
    /// read as user actions.
    #[test]
    fn autosync_reports_under_its_own_names() -> Result {
        use crate::telemetry::event::{AutosyncEvent, AutosyncPausedEvent, PausedKind};

        let (name, props) = event_payload(&MixpanelEvent::AutosyncPublished(AutosyncEvent {
            host: host(),
        }))?;
        assert_eq!(name, "autosync_published");
        assert_eq!(props.expect("host").get("host"), Some(&host_value()));

        let (name, props) = event_payload(&MixpanelEvent::AutosyncPaused(AutosyncPausedEvent {
            host: host(),
            reason: PausedKind::RoleDenied,
        }))?;
        assert_eq!(name, "autosync_paused");
        let props = props.expect("host and reason");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(
            props.get("reason"),
            Some(&Value::String("role_denied".to_owned()))
        );

        Ok(())
    }

    /// A pause carries the *category* and nothing else — no file names, no role
    /// name, no message. The engine's own reason type holds all three for the UI
    /// banner, and none of it may cross into the vocabulary.
    #[test]
    fn a_pause_carries_no_free_text() -> Result {
        use crate::autopull::PausedReason;
        use crate::telemetry::event::{AutosyncPausedEvent, PausedKind};

        for reason in [
            PausedReason::PullConflict(vec!["secret-project/notes.txt".to_owned()]),
            PausedReason::RoleDenied {
                role: "an-internal-role-name".to_owned(),
            },
            PausedReason::Other("a raw refusal mentioning /home/someone/path".to_owned()),
        ] {
            let (_, props) = event_payload(&MixpanelEvent::AutosyncPaused(AutosyncPausedEvent {
                host: host(),
                reason: PausedKind::from(&reason),
            }))?;

            let rendered = serde_json::to_string(&props.expect("payload"))?;
            assert!(
                !rendered.contains("notes.txt")
                    && !rendered.contains("an-internal-role-name")
                    && !rendered.contains("/home/someone"),
                "detail leaked from {reason:?} into {rendered}"
            );
        }

        Ok(())
    }

    /// The login-required event's host is optional, and an absent one is reported
    /// rather than dropped — the same rule the unattributed package events follow.
    #[test]
    fn an_unattributed_login_requirement_still_reports() -> Result {
        use crate::telemetry::event::AutosyncAuthEvent;

        let (name, props) =
            event_payload(&MixpanelEvent::AutosyncLoginRequired(AutosyncAuthEvent {
                host: None,
            }))?;

        assert_eq!(name, "autosync_login_required");
        assert_eq!(props.expect("payload").get("host"), Some(&Value::Null));

        Ok(())
    }

    /// The dry-run and off arms are reachable and never fail the caller — a
    /// telemetry sink must not be able to break the command it rides on. This
    /// exercises the arms; that the line reaches a terminal is not something a
    /// test can see.
    #[tokio::test]
    async fn the_silent_arms_never_fail_the_caller() -> Result {
        track_event(&Analytics::DryRun, &MixpanelEvent::AppLaunched, None).await?;
        track_event(&Analytics::Off, &MixpanelEvent::AppLaunched, None).await?;
        Ok(())
    }

    /// A dry run is only useful if what it prints is what would have been sent.
    /// These assert the line carries the wire *name* and the wire *properties* —
    /// so a developer reading the console is reading the payload, and a payload
    /// bug is visible there rather than hidden behind a friendlier rendering.
    #[test]
    fn dry_run_reports_the_wire_name_and_properties() -> Result {
        let line = dry_run_line(
            &MixpanelEvent::UserLoggedIn(LoginEvent {
                host: host(),
                flow: LoginFlow::OAuth,
            }),
            None,
        )?;

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
        let line = dry_run_line(&MixpanelEvent::AppLaunched, None)?;

        assert!(line.ends_with("app_launched"), "unexpected trailer: {line}");

        Ok(())
    }

    /// An unattributed event still prints, with an explicit null — the emitter
    /// failed to supply context and that must be readable, not invisible.
    #[test]
    fn dry_run_shows_an_absent_host_as_null() -> Result {
        let line = dry_run_line(
            &MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(None)),
            None,
        )?;

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
        // Autosync's host is guaranteed, so the accessor must find it — an
        // unattributed autosync event would be a contradiction.
        assert_eq!(
            MixpanelEvent::AutosyncPublished(crate::telemetry::event::AutosyncEvent {
                host: host()
            })
            .host(),
            Some(&host())
        );
        assert_eq!(
            MixpanelEvent::PackageCommitted(PackageEvent::for_uri(None)).host(),
            None
        );
    }
}

//! Sending events to Mixpanel. *What* we report lives in
//! [`event`](super::event).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use mixpanel_rs::{Config, Event, Mixpanel};
use quilt_uri::Host;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::env;
use crate::error::TelemetryError;
use crate::telemetry::{InstallId, MixpanelEvent, Sinks};

/// How many events may wait to be sent before the queue starts refusing.
///
/// Bounded on purpose: an unbounded queue turns a stuck sender into unbounded
/// memory. Large enough that a burst of user actions never reaches it, small
/// enough that a genuinely stuck sender is a bounded leak.
pub const QUEUE_DEPTH: usize = 1024;

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
            // Without this the client does not ask the ingest API to say whether
            // it *accepted* the event, so it never reads the status field and a
            // rejected event returns HTTP 200 and is reported as a success. A
            // malformed property would fail silently, forever.
            verbose: true,
            ..Default::default()
        };
        (token, config)
    })
}

/// Whether a failed send means **we** are wrong, or the network was.
///
/// The distinction decides whether a fault is worth reporting. A refusal is a bug
/// in what we sent — a bad token, a property the API will not take — and it fails
/// *every* event until someone fixes it, so it must be visible. A transport
/// failure is somebody's wifi; reporting those would fill the crash reporter with
/// other people's tunnels and bury the refusals among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailure {
    /// The ingest API took the request and refused the event.
    Refused,
    /// The request never got a verdict.
    Unreachable,
}

impl DeliveryFailure {
    pub fn classify(err: &crate::Error) -> Self {
        use mixpanel_rs::error::Error as Mp;

        let crate::Error::Telemetry(err) = err else {
            // Nothing outside telemetry reaches this, and an unrecognised failure
            // is safer treated as ours than as the network's.
            return Self::Refused;
        };

        // Exhaustive over our own errors as well as the client's, so a new one
        // cannot inherit an answer by falling through. That is precisely how a
        // timeout once read as a refusal.
        let err = match err {
            // A timeout *did* reach the network — it simply got no verdict, which
            // is the user's connection rather than our payload.
            TelemetryError::SendTimeout(_) => return Self::Unreachable,
            // A payload we could not even serialize never left the process.
            TelemetryError::Serialize(_) => return Self::Refused,
            TelemetryError::Mixpanel(err) => err,
        };

        match err {
            // A verdict came back, and it was no. `ApiClientError` is what the
            // verbose response parses a non-`1` status into.
            Mp::ApiClientError(..)
            | Mp::ApiUnexpectedResponse(_)
            | Mp::ApiPayloadTooLarge
            | Mp::JsonError(_)
            | Mp::UrlError(_)
            | Mp::TimeError => Self::Refused,

            // No verdict: the request failed, was throttled, or the server was
            // unwell. Retried by the client first; this is what is left after.
            Mp::HttpError(_)
            | Mp::ApiServerError(_)
            | Mp::ApiRateLimitError(_)
            | Mp::ApiHttpError(..)
            | Mp::MaxRetriesReached(_) => Self::Unreachable,
        }
    }
}

/// How many events one request carries at most.
///
/// The ingest API's own ceiling is higher; this is about latency, not its limit.
/// A queue that has fallen behind should catch up in several requests rather than
/// one enormous one, so a single failure loses less.
const BATCH: usize = 25;

/// How long one request may take before it is abandoned.
///
/// Wraps the send rather than configuring the client, because the analytics client
/// builds its own HTTP client with no timeout and exposes no knob for one. Without
/// this a hung socket holds a queued batch forever — which used to hold a *user's
/// click* forever, before emission moved off the request path.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// An event on its way out, stamped at the moment it happened.
///
/// The stamp is the point. Once events are queued, the API's receipt time is no
/// longer when the user acted — a batch sent after a delay would report the delay.
/// The idempotency key is the same argument for retries: the client retries a
/// failed request, so without a key a delivered-but-unacknowledged batch counts
/// twice.
pub struct Queued {
    event: MixpanelEvent,
    at: SystemTime,
    insert_id: Uuid,
}

impl Queued {
    pub fn now(event: MixpanelEvent) -> Self {
        Self {
            event,
            at: SystemTime::now(),
            insert_id: Uuid::new_v4(),
        }
    }

    pub fn host(&self) -> Option<&Host> {
        self.event.host()
    }

    /// The wire form, with the queue's own two properties merged in beside the
    /// event's and the install's.
    fn wire(&self, install_id: Option<&InstallId>) -> crate::Result<Event> {
        let (event, properties) = wire_payload(&self.event, install_id)?;
        let mut properties = properties.unwrap_or_default();

        // Seconds since the epoch is what the ingest API reads `time` as. A clock
        // before 1970 is not worth a failure path: falling back to omitting the
        // stamp lets the server date it, which is what happened before anyway.
        if let Ok(since_epoch) = self.at.duration_since(SystemTime::UNIX_EPOCH) {
            properties.insert("time".to_owned(), Value::from(since_epoch.as_secs()));
        }
        properties.insert(
            "$insert_id".to_owned(),
            Value::String(self.insert_id.to_string()),
        );

        Ok(Event { event, properties })
    }
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

/// Drain the queue until the sender is dropped, sending what it holds in batches.
///
/// **This is the single consumer** of the channel — the "sc" of `mpsc`. One task is
/// what makes requests never overlap and order survive; a second would give both
/// away.
///
/// `recv` *awaits* the first event of a batch, so an idle app costs nothing rather
/// than polling. `try_recv` then takes whatever else is already waiting without
/// waiting for more, so a quiet app sends one event immediately instead of holding
/// it for a batch that may never fill, while a busy one coalesces.
///
/// The loop ends when every sender is dropped and the channel closes, which is how
/// the task retires at shutdown rather than needing to be told.
///
/// Failures are handed to `on_failure` rather than reported here, because deciding
/// what a failure deserves is not this loop's business — see
/// [`DeliveryFailure`].
pub async fn run_sender(
    analytics: Analytics,
    install_id: Option<InstallId>,
    mut queue: mpsc::Receiver<Queued>,
    on_failure: impl Fn(&crate::Error),
) {
    while let Some(first) = queue.recv().await {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match queue.try_recv() {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }

        if let Err(err) = send_batch(&analytics, install_id.as_ref(), batch).await {
            on_failure(&err);
        }
    }
}

async fn send_batch(
    analytics: &Analytics,
    install_id: Option<&InstallId>,
    batch: Vec<Queued>,
) -> crate::Result<()> {
    match analytics {
        Analytics::Live(mixpanel) => {
            let events = batch
                .iter()
                .map(|queued| queued.wire(install_id))
                .collect::<crate::Result<Vec<_>>>()?;

            tokio::time::timeout(SEND_TIMEOUT, mixpanel.track_batch(events))
                .await
                .map_err(|_| TelemetryError::SendTimeout(SEND_TIMEOUT.as_secs()))??;
        }
        Analytics::DryRun => {
            for queued in &batch {
                eprintln!("{}", dry_run_line(&queued.event, install_id)?);
            }
        }
        Analytics::Off => {}
    }
    Ok(())
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

    /// Queueing is what creates the need for a client-side stamp: without it a
    /// batch sent after a delay is dated by its arrival, so the delay reads as when
    /// the user acted. The idempotency key is the same argument for the client's
    /// own retries.
    #[test]
    fn a_queued_event_carries_its_own_time_and_an_idempotency_key() -> Result {
        let before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("a clock after 1970")
            .as_secs();

        let queued = Queued::now(MixpanelEvent::AppLaunched);
        let wire = queued.wire(None)?;

        let stamped = wire
            .properties
            .get("time")
            .and_then(Value::as_u64)
            .expect("a client-side stamp");
        assert!(
            stamped >= before,
            "stamped {stamped} before the test started at {before}"
        );

        let key = wire
            .properties
            .get("$insert_id")
            .and_then(Value::as_str)
            .expect("an idempotency key");
        assert!(Uuid::parse_str(key).is_ok(), "not a uuid: {key}");

        Ok(())
    }

    /// Two events are two keys, or the ingest API would fold them into one.
    #[test]
    fn each_event_gets_its_own_idempotency_key() -> Result {
        let one = Queued::now(MixpanelEvent::AppLaunched).wire(None)?;
        let other = Queued::now(MixpanelEvent::AppLaunched).wire(None)?;

        assert_ne!(
            one.properties.get("$insert_id"),
            other.properties.get("$insert_id")
        );

        Ok(())
    }

    /// The queue's properties sit beside the event's and the install's rather than
    /// replacing them — four facts from three owners, all on one event.
    #[test]
    fn queueing_does_not_displace_the_event_or_the_identity() -> Result {
        let id = install_id();

        let wire = Queued::now(MixpanelEvent::UserLoggedIn(LoginEvent {
            host: host(),
            flow: LoginFlow::OAuth,
        }))
        .wire(Some(&id))?;

        assert_eq!(wire.event, "user_logged_in");
        assert_eq!(wire.properties.get("host"), Some(&host_value()));
        assert_eq!(
            wire.properties.get("distinct_id"),
            Some(&Value::String(id.as_str().to_owned()))
        );
        assert!(wire.properties.contains_key("time"));
        assert!(wire.properties.contains_key("$insert_id"));

        Ok(())
    }

    /// The sender drains what is waiting and stops when the queue closes, rather
    /// than spinning or hanging — a dropped sender must end the task.
    #[tokio::test]
    async fn the_sender_drains_the_queue_and_stops_when_it_closes() {
        let (queue, receiver) = mpsc::channel(QUEUE_DEPTH);

        for _ in 0..3 {
            queue
                .try_send(Queued::now(MixpanelEvent::AppLaunched))
                .expect("room in the queue");
        }
        drop(queue);

        let failures = std::sync::Mutex::new(Vec::new());
        run_sender(Analytics::Off, None, receiver, |err| {
            failures.lock().unwrap().push(err.to_string());
        })
        .await;

        assert!(
            failures.lock().unwrap().is_empty(),
            "the off sink cannot fail: {:?}",
            failures.lock().unwrap()
        );
    }

    /// A verdict from the API is our bug; no verdict is somebody's network. The
    /// split decides what reaches the crash reporter, so it is worth pinning per
    /// variant rather than by category.
    #[test]
    fn a_refusal_and_an_unreachable_api_are_told_apart() {
        use mixpanel_rs::error::Error as Mp;

        let refused = [
            Mp::ApiClientError(400, "no such token".to_owned()),
            Mp::ApiUnexpectedResponse("not json".to_owned()),
            Mp::ApiPayloadTooLarge,
        ];
        for err in refused {
            let err = crate::Error::from(err);
            assert_eq!(
                DeliveryFailure::classify(&err),
                DeliveryFailure::Refused,
                "the API answered, and said no: {err}"
            );
        }

        let unreachable = [
            Mp::ApiServerError(503),
            Mp::ApiRateLimitError(Some(30)),
            Mp::MaxRetriesReached("gave up".to_owned()),
            Mp::ApiHttpError(502, "bad gateway".to_owned()),
        ];
        for err in unreachable {
            let err = crate::Error::from(err);
            assert_eq!(
                DeliveryFailure::classify(&err),
                DeliveryFailure::Unreachable,
                "no verdict came back: {err}"
            );
        }
    }

    /// A failure that never reached the network is ours by definition — a payload
    /// we could not even serialize is not the user's connection.
    #[test]
    fn a_serialization_failure_counts_as_a_refusal() {
        let err = crate::Error::from(TelemetryError::Serialize("bad shape".to_owned()));
        assert_eq!(DeliveryFailure::classify(&err), DeliveryFailure::Refused);
    }

    /// A timeout *did* reach the network and got no verdict, so it is the user's
    /// connection, not our payload.
    ///
    /// This regressed once and the existing coverage did not catch it: the timeout
    /// borrowed the serialization error, which classifies as a refusal, so a slow
    /// network filed a false fault **and spent the one refusal report a run is
    /// allowed** — swallowing any genuine refusal later in the same run. The
    /// per-variant tests passed throughout, because the variant they exercised was
    /// not the one the timeout used.
    #[test]
    fn a_timeout_is_the_network_not_our_payload() {
        let err = crate::Error::from(TelemetryError::SendTimeout(10));
        assert_eq!(
            DeliveryFailure::classify(&err),
            DeliveryFailure::Unreachable,
            "a timeout must not be reported as our bug"
        );
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
        send_batch(
            &Analytics::DryRun,
            None,
            vec![Queued::now(MixpanelEvent::AppLaunched)],
        )
        .await?;
        send_batch(
            &Analytics::Off,
            None,
            vec![Queued::now(MixpanelEvent::AppLaunched)],
        )
        .await?;
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

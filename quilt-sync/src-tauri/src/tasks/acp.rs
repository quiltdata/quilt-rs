use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use agent_client_protocol::Agent;
use agent_client_protocol::ByteStreams;
use agent_client_protocol::Client;
use agent_client_protocol::ConnectTo;
use agent_client_protocol::ConnectionTo;
use agent_client_protocol::ErrorCode;
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::ContentBlock;
use agent_client_protocol::schema::v1::InitializeRequest;
use agent_client_protocol::schema::v1::NewSessionRequest;
use agent_client_protocol::schema::v1::PermissionOptionKind;
use agent_client_protocol::schema::v1::PromptRequest;
use agent_client_protocol::schema::v1::RequestPermissionOutcome;
use agent_client_protocol::schema::v1::RequestPermissionRequest;
use agent_client_protocol::schema::v1::RequestPermissionResponse;
use agent_client_protocol::schema::v1::SelectedPermissionOutcome;
use agent_client_protocol::schema::v1::StopReason;
use agent_client_protocol::schema::v1::TextContent;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::tasks::settings::AgentSettings;

const SIGNED_OUT: &str = "agent not signed in";

#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The claim stopped being `latest` before the prompt.
    Superseded,
    NotStarted {
        reason: String,
    },
    /// The task changed under the claim into one that can't run as written.
    Unrunnable {
        reason: String,
    },
    Done {
        session_id: String,
        stop_reason: String,
    },
    Failed {
        session_id: Option<String>,
        stop_reason: Option<String>,
        reason: Option<String>,
    },
}

/// `recheck` re-reads the task after the session opens, and may bring a later
/// revision down: the document to prompt, or how the run ends unprompted.
pub async fn run(
    agent: &AgentSettings,
    cwd: &Path,
    recheck: impl AsyncFnOnce() -> Result<String, RunOutcome>,
) -> RunOutcome {
    let Some((program, args)) = agent.command.split_first() else {
        return RunOutcome::NotStarted {
            reason: "the agent's command is empty".to_string(),
        };
    };
    let spawned = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            return RunOutcome::NotStarted {
                reason: format!("could not start {program}: {err}"),
            };
        }
    };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return RunOutcome::NotStarted {
            reason: "the agent's stdio could not be opened".to_string(),
        };
    };
    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());
    let outcome = converse(transport, cwd, agent.trust_all, recheck).await;
    let _ = child.kill().await;
    outcome
}

async fn converse(
    transport: impl ConnectTo<Client> + 'static,
    cwd: &Path,
    trust_all: bool,
    recheck: impl AsyncFnOnce() -> Result<String, RunOutcome>,
) -> RunOutcome {
    let root = cwd.to_path_buf();
    let prompted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prompting = prompted.clone();
    let connected = Client
        .builder()
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                responder.respond(answer(&request, trust_all, &root))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async |cx: ConnectionTo<Agent>| {
            Ok(session(&cx, cwd, recheck, &prompting).await)
        })
        .await;
    connected.unwrap_or_else(|err| {
        connection_failed(prompted.load(std::sync::atomic::Ordering::SeqCst), &err)
    })
}

fn connection_failed(prompted: bool, err: &agent_client_protocol::Error) -> RunOutcome {
    if prompted && !signed_out(err) {
        failed(None, err)
    } else {
        not_started(err)
    }
}

async fn session(
    cx: &ConnectionTo<Agent>,
    cwd: &Path,
    recheck: impl AsyncFnOnce() -> Result<String, RunOutcome>,
    prompted: &std::sync::atomic::AtomicBool,
) -> RunOutcome {
    // The default capabilities advertise no file system and no terminal.
    let initialize = InitializeRequest::new(ProtocolVersion::V1);
    if let Err(err) = cx.send_request(initialize).block_task().await {
        return not_started(&err);
    }
    let session_id = match cx
        .send_request(NewSessionRequest::new(cwd))
        .block_task()
        .await
    {
        Ok(session) => session.session_id,
        Err(err) => return not_started(&err),
    };
    let document = match recheck().await {
        Ok(document) => document,
        Err(outcome) => return outcome,
    };
    prompted.store(true, std::sync::atomic::Ordering::SeqCst);
    let prompt = PromptRequest::new(
        session_id.clone(),
        vec![ContentBlock::Text(TextContent::new(format!(
            "Do the task described in {document}, in this folder."
        )))],
    );
    let ended = cx.send_request(prompt).block_task().await;
    turn_ended(session_id.to_string(), ended.map(|r| r.stop_reason))
}

fn signed_out(err: &agent_client_protocol::Error) -> bool {
    err.code == ErrorCode::AuthRequired
}

fn not_started(err: &agent_client_protocol::Error) -> RunOutcome {
    RunOutcome::NotStarted {
        reason: if signed_out(err) {
            SIGNED_OUT.to_string()
        } else {
            err.to_string()
        },
    }
}

fn failed(session_id: Option<String>, err: &agent_client_protocol::Error) -> RunOutcome {
    RunOutcome::Failed {
        session_id,
        stop_reason: None,
        reason: Some(err.to_string()),
    }
}

fn turn_ended(
    session_id: String,
    ended: Result<StopReason, agent_client_protocol::Error>,
) -> RunOutcome {
    match ended {
        Ok(StopReason::EndTurn) => RunOutcome::Done {
            session_id,
            stop_reason: "end_turn".to_string(),
        },
        Ok(other) => RunOutcome::Failed {
            session_id: Some(session_id),
            stop_reason: Some(stop_reason_name(other)),
            reason: None,
        },
        Err(err) if signed_out(&err) => not_started(&err),
        Err(err) => failed(Some(session_id), &err),
    }
}

/// The stop reason as the protocol spells it, such as `end_turn`.
fn stop_reason_name(reason: StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{reason:?}"))
}

fn answer(
    request: &RequestPermissionRequest,
    trust_all: bool,
    root: &Path,
) -> RequestPermissionResponse {
    let locations: Vec<&Path> = request
        .tool_call
        .fields
        .locations
        .iter()
        .flatten()
        .map(|location| location.path.as_path())
        .collect();
    // "Allow always" only under `trust_all`: otherwise it would pass the agent's
    // later requests unchecked. A request with no option to allow is refused,
    // and `Cancelled`, which ends the turn, only when no option refuses it.
    let refuse = [
        PermissionOptionKind::RejectOnce,
        PermissionOptionKind::RejectAlways,
    ];
    let allow: &[_] = match (permits(trust_all, root, &locations), trust_all) {
        (false, _) => &[],
        (true, false) => &[PermissionOptionKind::AllowOnce],
        (true, true) => &[
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ],
    };
    let chosen = allow
        .iter()
        .chain(&refuse)
        .find_map(|kind| request.options.iter().find(|option| option.kind == *kind));
    let outcome = match chosen {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
    };
    RequestPermissionResponse::new(outcome)
}

fn permits(trust_all: bool, root: &Path, locations: &[&Path]) -> bool {
    let root = normalise(root);
    trust_all
        || (!locations.is_empty()
            && locations
                .iter()
                .all(|path| normalise(&root.join(path)).starts_with(&root)))
}

/// `path` with `.` and `..` resolved lexically.
fn normalise(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_client_protocol::Channel;
    use agent_client_protocol::schema::v1::InitializeResponse;
    use agent_client_protocol::schema::v1::NewSessionResponse;
    use agent_client_protocol::schema::v1::PermissionOption;
    use agent_client_protocol::schema::v1::PromptResponse;
    use agent_client_protocol::schema::v1::ToolCallLocation;
    use agent_client_protocol::schema::v1::ToolCallUpdate;
    use agent_client_protocol::schema::v1::ToolCallUpdateFields;

    const ROOT: &str = "/home/u/QuiltSync/acme/demo";

    fn permitted(trust_all: bool, locations: &[&str]) -> bool {
        let locations: Vec<&Path> = locations.iter().map(Path::new).collect();
        permits(trust_all, Path::new(ROOT), &locations)
    }

    #[test]
    fn trust_all_approves_every_request() {
        assert!(permitted(true, &[]));
        assert!(permitted(true, &["/etc/hosts"]));
    }

    #[test]
    fn a_request_inside_the_working_directory_is_approved() {
        assert!(permitted(false, &[&format!("{ROOT}/out/report.md")]));
        assert!(permitted(false, &["out/report.md", "sub/../TASK.md"]));
    }

    #[test]
    fn a_request_naming_an_outside_path_or_none_is_refused() {
        assert!(!permitted(false, &["/etc/hosts"]));
        assert!(!permitted(false, &["out/ok.md", "../other/file"]));
        assert!(!permitted(false, &[&format!("{ROOT}-sibling/file")]));
        assert!(!permitted(false, &[]));
    }

    fn options() -> Vec<PermissionOption> {
        [
            ("accept", PermissionOptionKind::AllowOnce),
            ("always-accept", PermissionOptionKind::AllowAlways),
            ("reject", PermissionOptionKind::RejectOnce),
            ("always-reject", PermissionOptionKind::RejectAlways),
        ]
        .into_iter()
        .map(|(id, kind)| PermissionOption::new(id, id, kind))
        .collect()
    }

    fn permission_request(locations: &[&str]) -> RequestPermissionRequest {
        let fields = ToolCallUpdateFields::new().locations(
            locations
                .iter()
                .map(|path| ToolCallLocation::new(*path))
                .collect::<Vec<_>>(),
        );
        RequestPermissionRequest::new("s1", ToolCallUpdate::new("call-1", fields), options())
    }

    fn chosen(response: &RequestPermissionResponse) -> Option<String> {
        match &response.outcome {
            RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.to_string()),
            _ => None,
        }
    }

    #[test]
    fn the_answer_picks_the_once_option_of_its_kind() {
        let root = Path::new(ROOT);
        let inside = answer(&permission_request(&["out.md"]), false, root);
        assert_eq!(chosen(&inside).as_deref(), Some("accept"));
        let outside = answer(&permission_request(&["/etc/hosts"]), false, root);
        assert_eq!(chosen(&outside).as_deref(), Some("reject"));
    }

    #[test]
    fn with_only_always_options_a_request_is_allowed_for_good_only_under_trust_all() {
        let mut request = permission_request(&["out.md"]);
        request.options.retain(|option| {
            !matches!(
                option.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::RejectOnce
            )
        });
        assert_eq!(
            chosen(&answer(&request, false, Path::new(ROOT))).as_deref(),
            Some("always-reject")
        );
        assert_eq!(
            chosen(&answer(&request, true, Path::new(ROOT))).as_deref(),
            Some("always-accept")
        );
    }

    fn auth_required() -> agent_client_protocol::Error {
        agent_client_protocol::Error::new(
            -32000,
            "Kiro could not load the available models because you are not signed in",
        )
    }

    #[test]
    fn end_turn_is_done_and_any_other_stop_is_failed() {
        assert_eq!(
            turn_ended("s1".to_string(), Ok(StopReason::EndTurn)),
            RunOutcome::Done {
                session_id: "s1".to_string(),
                stop_reason: "end_turn".to_string()
            }
        );
        assert_eq!(
            turn_ended("s1".to_string(), Ok(StopReason::MaxTokens)),
            RunOutcome::Failed {
                session_id: Some("s1".to_string()),
                stop_reason: Some("max_tokens".to_string()),
                reason: None
            }
        );
        assert!(matches!(
            turn_ended(
                "s1".to_string(),
                Err(agent_client_protocol::Error::internal_error())
            ),
            RunOutcome::Failed {
                stop_reason: None,
                reason: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn auth_required_anywhere_is_not_started_signed_out() {
        let signed_out = RunOutcome::NotStarted {
            reason: SIGNED_OUT.to_string(),
        };
        assert_eq!(not_started(&auth_required()), signed_out);
        assert_eq!(
            turn_ended("s1".to_string(), Err(auth_required())),
            signed_out
        );
        assert_eq!(connection_failed(true, &auth_required()), signed_out);
        assert!(matches!(
            connection_failed(true, &agent_client_protocol::Error::internal_error()),
            RunOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn an_agent_that_cannot_be_spawned_is_not_started() {
        let agent = AgentSettings {
            command: vec!["/nonexistent/quiltsync-test-agent".to_string()],
            trust_all: false,
        };
        let outcome = run(&agent, Path::new("/"), async || Ok("TASK.md".to_string())).await;
        assert!(
            matches!(outcome, RunOutcome::NotStarted { .. }),
            "{outcome:?}"
        );
    }

    /// An in-process agent: it opens session `s1` and, prompted, asks to write
    /// `out.md` in the folder, then ends its turn. It reports the prompt it got
    /// and the option the client picked.
    fn fake_agent(cwd: &Path) -> (Channel, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (client_end, agent_end) = Channel::duplex();
        let (seen, heard) = tokio::sync::mpsc::unbounded_channel();
        let out = cwd.join("out.md");
        let prompted = seen.clone();
        tokio::spawn(
            Agent
                .builder()
                .on_receive_request(
                    async move |request: InitializeRequest, responder, _cx| {
                        responder.respond(InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: NewSessionRequest, responder, _cx| {
                        responder.respond(NewSessionResponse::new("s1"))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                        let _ = prompted.send(format!("{:?}", request.prompt));
                        let fields = ToolCallUpdateFields::new()
                            .locations(vec![ToolCallLocation::new(out.clone())]);
                        let ask = RequestPermissionRequest::new(
                            "s1",
                            ToolCallUpdate::new("write", fields),
                            options(),
                        );
                        let picked = seen.clone();
                        let asking = cx.clone();
                        cx.spawn(async move {
                            let answer = asking.send_request(ask).block_task().await?;
                            let _ = picked.send(chosen(&answer).unwrap_or_default());
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(agent_end),
        );
        (client_end, heard)
    }

    #[tokio::test]
    async fn a_session_prompts_the_document_and_ends_done() {
        let cwd = Path::new(ROOT);
        let (transport, mut heard) = fake_agent(cwd);
        let outcome = converse(transport, cwd, false, async || Ok("TASK.md".to_string())).await;
        assert_eq!(
            outcome,
            RunOutcome::Done {
                session_id: "s1".to_string(),
                stop_reason: "end_turn".to_string()
            }
        );
        let prompt = heard.recv().await.expect("prompted");
        assert!(prompt.contains("TASK.md"), "{prompt}");
        assert_eq!(
            heard.recv().await.as_deref(),
            Some("accept"),
            "a write inside the folder is approved"
        );
    }

    #[tokio::test]
    async fn a_claim_that_lost_latest_is_never_prompted() {
        let cwd = Path::new(ROOT);
        let (transport, mut heard) = fake_agent(cwd);
        let outcome = converse(transport, cwd, false, async || Err(RunOutcome::Superseded)).await;
        assert_eq!(outcome, RunOutcome::Superseded);
        assert_eq!(heard.try_recv().ok(), None, "no prompt was sent");
    }
}

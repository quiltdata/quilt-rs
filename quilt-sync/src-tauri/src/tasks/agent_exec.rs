//! The `agent_exec` key of a package revision's metadata: what it asks this
//! desktop to do, and the claim each revision of a run writes back.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use serde_json::json;

use crate::tasks::acp::RunOutcome;
use crate::tasks::settings::AgentSettings;

#[derive(Debug, PartialEq)]
pub enum Verdict {
    LeaveAlone,
    Claim {
        document: String,
        agent: String,
    },
    Unrunnable {
        reason: String,
    },
    /// This desktop's own claim, with no run alive to finish it.
    Interrupted,
}

/// Read the head revision's `user_meta`. `is_file` says whether a path is a
/// file in the revision's manifest; `own` holds this desktop's invocation IDs.
///
/// Called only for a package with no live run here, so a claim of this
/// desktop's own is one a restart cut short.
pub fn classify(
    user_meta: &Value,
    is_file: impl Fn(&Path) -> bool,
    agents: &BTreeMap<String, AgentSettings>,
    own: &BTreeSet<String>,
) -> Verdict {
    let Some(agent_exec) = user_meta.get("agent_exec") else {
        return Verdict::LeaveAlone;
    };
    let claim = agent_exec.get("claim");
    let Some(status) = claim.and_then(|c| c.get("status")).and_then(Value::as_str) else {
        return Verdict::LeaveAlone;
    };
    let is_own = claim
        .and_then(|c| c.get("invocation_id"))
        .and_then(Value::as_str)
        .is_some_and(|id| own.contains(id));
    match status {
        "unclaimed" => {}
        "not_started" if !is_own => {}
        "claimed" if is_own => return Verdict::Interrupted,
        _ => return Verdict::LeaveAlone,
    }

    let Some(agent) = agent_exec
        .get("agent")
        .and_then(|a| a.get("id"))
        .and_then(Value::as_str)
    else {
        return Verdict::Unrunnable {
            reason: "agent_exec.agent.id is missing".to_string(),
        };
    };
    if !agents.contains_key(agent) {
        return Verdict::LeaveAlone;
    }
    let Some(document) = agent_exec.get("document").and_then(Value::as_str) else {
        return Verdict::Unrunnable {
            reason: "agent_exec.document is missing".to_string(),
        };
    };
    if !is_file(Path::new(document)) {
        return Verdict::Unrunnable {
            reason: format!("{document} is not a file in the package"),
        };
    }
    Verdict::Claim {
        document: document.to_string(),
        agent: agent.to_string(),
    }
}

/// The claim one run writes in every revision it publishes.
#[derive(Debug)]
pub struct Claim {
    pub invocation_id: String,
    pub claimed_at: String,
}

impl Claim {
    pub fn claimed(&self) -> Value {
        json!({
            "status": "claimed",
            "invocation_id": self.invocation_id,
            "claimed_at": self.claimed_at,
        })
    }

    /// The claim the run's last revision carries; `None` when it publishes
    /// nothing.
    pub fn outcome(&self, outcome: &RunOutcome) -> Option<Value> {
        let mut claim = self.claimed();
        let mut set = |key: &str, value: Option<&String>| {
            if let Some(value) = value {
                claim[key] = json!(value);
            }
        };
        let status = match outcome {
            RunOutcome::Superseded => return None,
            RunOutcome::NotStarted { reason } => {
                set("reason", Some(reason));
                "not_started"
            }
            RunOutcome::Done {
                session_id,
                stop_reason,
            } => {
                set("session_id", Some(session_id));
                set("stop_reason", Some(stop_reason));
                "done"
            }
            RunOutcome::Failed {
                session_id,
                stop_reason,
                reason,
            } => {
                set("session_id", session_id.as_ref());
                set("stop_reason", stop_reason.as_ref());
                set("reason", reason.as_ref());
                "failed"
            }
        };
        claim["status"] = json!(status);
        Some(claim)
    }
}

/// `user_meta` with `agent_exec.claim` replaced by `claim` and nothing else
/// changed.
pub fn with_claim(user_meta: &Value, claim: Value) -> Value {
    let mut meta = user_meta.clone();
    meta["agent_exec"]["claim"] = claim;
    meta
}

/// `user_meta`'s claim marked failed by an interruption, keeping its
/// invocation ID and claim time.
pub fn interrupted(user_meta: &Value) -> Value {
    let mut claim = user_meta["agent_exec"]["claim"].clone();
    claim["status"] = json!("failed");
    claim["reason"] = json!("interrupted by restart");
    with_claim(user_meta, claim)
}

pub fn unrunnable(user_meta: &Value, reason: &str) -> Value {
    with_claim(
        user_meta,
        json!({ "status": "unrunnable", "reason": reason }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents() -> BTreeMap<String, AgentSettings> {
        BTreeMap::from([(
            "kiro".to_string(),
            AgentSettings {
                command: vec!["kiro-cli-chat".to_string()],
                trust_all: false,
            },
        )])
    }

    fn task(agent: &str, document: &str, claim: &Value) -> Value {
        json!({
            "other": "kept",
            "agent_exec": { "document": document, "agent": { "id": agent }, "claim": claim },
        })
    }

    fn verdict_for(meta: &Value, own: &[&str]) -> Verdict {
        let own = own.iter().map(ToString::to_string).collect();
        classify(meta, |path| path == Path::new("TASK.md"), &agents(), &own)
    }

    fn claim_kiro() -> Verdict {
        Verdict::Claim {
            document: "TASK.md".to_string(),
            agent: "kiro".to_string(),
        }
    }

    #[test]
    fn an_unclaimed_task_for_a_listed_agent_is_claimed() {
        let meta = task("kiro", "TASK.md", &json!({ "status": "unclaimed" }));
        assert_eq!(verdict_for(&meta, &[]), claim_kiro());
    }

    #[test]
    fn another_desktops_not_started_is_claimed_and_our_own_is_not() {
        let meta = task(
            "kiro",
            "TASK.md",
            &json!({ "status": "not_started", "invocation_id": "theirs" }),
        );
        assert_eq!(verdict_for(&meta, &["ours"]), claim_kiro());
        assert_eq!(verdict_for(&meta, &["theirs"]), Verdict::LeaveAlone);
    }

    #[test]
    fn our_own_claim_with_no_live_run_was_interrupted() {
        let meta = task(
            "kiro",
            "TASK.md",
            &json!({ "status": "claimed", "invocation_id": "ours" }),
        );
        assert_eq!(verdict_for(&meta, &["ours"]), Verdict::Interrupted);
        assert_eq!(verdict_for(&meta, &[]), Verdict::LeaveAlone);
    }

    #[test]
    fn a_task_for_an_agent_not_listed_here_is_left_for_another_desktop() {
        let meta = task("claude", "MISSING.md", &json!({ "status": "unclaimed" }));
        assert_eq!(verdict_for(&meta, &[]), Verdict::LeaveAlone);
    }

    #[test]
    fn a_document_not_in_the_package_is_unrunnable() {
        let meta = task("kiro", "MISSING.md", &json!({ "status": "unclaimed" }));
        assert_eq!(
            verdict_for(&meta, &[]),
            Verdict::Unrunnable {
                reason: "MISSING.md is not a file in the package".to_string()
            }
        );
    }

    #[test]
    fn a_claimable_task_missing_its_agent_or_document_is_unrunnable() {
        let no_agent =
            json!({ "agent_exec": { "document": "TASK.md", "claim": { "status": "unclaimed" } } });
        assert!(matches!(
            verdict_for(&no_agent, &[]),
            Verdict::Unrunnable { .. }
        ));
        let no_document = json!({ "agent_exec": { "agent": { "id": "kiro" }, "claim": { "status": "unclaimed" } } });
        assert!(matches!(
            verdict_for(&no_document, &[]),
            Verdict::Unrunnable { .. }
        ));
    }

    #[test]
    fn a_status_this_desktop_does_not_act_on_is_left_alone() {
        for claim in [
            json!({ "status": "done", "invocation_id": "ours" }),
            json!({ "status": "failed", "invocation_id": "ours" }),
            json!({ "status": "unrunnable" }),
            json!({ "status": "cancelled" }),
            json!({ "status": "someday" }),
            json!({}),
        ] {
            let meta = task("kiro", "TASK.md", &claim);
            assert_eq!(
                verdict_for(&meta, &["ours"]),
                Verdict::LeaveAlone,
                "{claim}"
            );
        }
        assert_eq!(
            verdict_for(&json!({ "other": 1 }), &[]),
            Verdict::LeaveAlone
        );
    }

    fn claim() -> Claim {
        Claim {
            invocation_id: "inv".to_string(),
            claimed_at: "2026-10-24T12:00:00Z".to_string(),
        }
    }

    #[test]
    fn a_turn_that_ended_normally_is_done_under_the_same_claim() {
        let outcome = RunOutcome::Done {
            session_id: "s1".to_string(),
            stop_reason: "end_turn".to_string(),
        };
        assert_eq!(
            claim().outcome(&outcome),
            Some(json!({
                "status": "done",
                "invocation_id": "inv",
                "claimed_at": "2026-10-24T12:00:00Z",
                "session_id": "s1",
                "stop_reason": "end_turn",
            }))
        );
    }

    #[test]
    fn a_run_that_did_not_succeed_is_failed_with_what_ended_it() {
        let outcome = RunOutcome::Failed {
            session_id: Some("s1".to_string()),
            stop_reason: Some("max_tokens".to_string()),
            reason: None,
        };
        let failed = claim().outcome(&outcome).expect("published");
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["stop_reason"], "max_tokens");
        assert_eq!(failed["invocation_id"], "inv");
        assert!(failed.get("reason").is_none());
    }

    #[test]
    fn an_agent_that_never_started_is_not_started_with_the_reason() {
        let outcome = RunOutcome::NotStarted {
            reason: "agent not signed in".to_string(),
        };
        let not_started = claim().outcome(&outcome).expect("published");
        assert_eq!(not_started["status"], "not_started");
        assert_eq!(not_started["reason"], "agent not signed in");
        assert_eq!(not_started["claimed_at"], "2026-10-24T12:00:00Z");
    }

    #[test]
    fn a_superseded_claim_publishes_nothing() {
        assert_eq!(claim().outcome(&RunOutcome::Superseded), None);
    }

    #[test]
    fn an_interruption_keeps_the_claim_it_fails() {
        let meta = task(
            "kiro",
            "TASK.md",
            &json!({ "status": "claimed", "invocation_id": "inv", "claimed_at": "t" }),
        );
        let failed = interrupted(&meta);
        assert_eq!(
            failed["agent_exec"]["claim"],
            json!({ "status": "failed", "invocation_id": "inv", "claimed_at": "t", "reason": "interrupted by restart" })
        );
        assert_eq!(failed["other"], "kept");
    }
}

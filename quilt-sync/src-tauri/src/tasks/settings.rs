use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::Error;
use crate::tasks::agent_process::Spawned;

const FILE_NAME: &str = "task_settings.json";
const INVOCATIONS_FILE_NAME: &str = "task_invocations.json";
const OUTCOMES_FILE_NAME: &str = "task_outcomes.json";
const AGENTS_FILE_NAME: &str = "task_agents.json";

/// Which buckets may hand this desktop a task, and which agents it can run.
///
/// Read from `task_settings.json` in `app_local_data_dir`, written by hand: the
/// app has no screen for it.
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct TaskSettings {
    #[serde(default)]
    pub blessed_buckets: Vec<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentSettings>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentSettings {
    /// The program and its arguments, run with the package's folder as cwd.
    pub command: Vec<String>,
    /// Approve every permission request the agent makes.
    #[serde(default)]
    pub trust_all: bool,
    /// From spawning the agent until `session/new` answers.
    pub startup_timeout_secs: Option<u64>,
    /// The prompt turn.
    pub turn_timeout_secs: Option<u64>,
}

impl AgentSettings {
    pub fn startup_timeout(&self) -> Duration {
        Duration::from_secs(self.startup_timeout_secs.unwrap_or(120))
    }

    pub fn turn_timeout(&self) -> Duration {
        Duration::from_secs(self.turn_timeout_secs.unwrap_or(1800))
    }
}

impl TaskSettings {
    /// `None` when tasks are off: no file, or no blessed bucket.
    pub async fn load(data_dir: &Path) -> Result<Option<Self>, Error> {
        match tokio::fs::read(data_dir.join(FILE_NAME)).await {
            Ok(bytes) => {
                let settings: Self = serde_json::from_slice(&bytes)?;
                Ok(Some(settings).filter(|s| !s.blessed_buckets.is_empty()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::from(err)),
        }
    }

    /// Whether `bucket`, a bare bucket name, is listed, as `s3://name` or `name`.
    pub fn blesses(&self, bucket: &str) -> bool {
        self.blessed_buckets.iter().any(|blessed| {
            let name = blessed.strip_prefix("s3://").unwrap_or(blessed);
            name.trim_end_matches('/') == bucket
        })
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Invocations {
    invocation_ids: BTreeSet<String>,
}

/// Each run's last claim, by invocation ID.
#[derive(Serialize, Deserialize, Default)]
struct Outcomes {
    outcomes: BTreeMap<String, Value>,
}

/// Runs finish concurrently, so one writer at a time.
static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Error> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(Error::from(err)),
    }
}

async fn update<T: Serialize + DeserializeOwned + Default>(
    data_dir: &Path,
    file_name: &str,
    change: impl FnOnce(&mut T),
) -> Result<(), Error> {
    let _writing = WRITING.lock().await;
    let path = data_dir.join(file_name);
    let mut value = read(&path).await?;
    change(&mut value);
    tokio::fs::create_dir_all(data_dir).await?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(&value)?).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

pub async fn own_invocations(data_dir: &Path) -> Result<BTreeSet<String>, Error> {
    Ok(read::<Invocations>(&data_dir.join(INVOCATIONS_FILE_NAME))
        .await?
        .invocation_ids)
}

pub async fn remember_invocation(data_dir: &Path, invocation_id: &str) -> Result<(), Error> {
    update(
        data_dir,
        INVOCATIONS_FILE_NAME,
        |invocations: &mut Invocations| {
            invocations.invocation_ids.insert(invocation_id.to_string());
        },
    )
    .await
}

/// The claim a run publishes as its last revision, saved before the publish,
/// so the sweep can publish it if that publish never lands.
pub async fn remember_outcome(
    data_dir: &Path,
    invocation_id: &str,
    claim: &Value,
) -> Result<(), Error> {
    update(data_dir, OUTCOMES_FILE_NAME, |outcomes: &mut Outcomes| {
        outcomes
            .outcomes
            .insert(invocation_id.to_string(), claim.clone());
    })
    .await
}

pub async fn remembered_outcome(
    data_dir: &Path,
    invocation_id: &str,
) -> Result<Option<Value>, Error> {
    let mut outcomes: Outcomes = read(&data_dir.join(OUTCOMES_FILE_NAME)).await?;
    Ok(outcomes.outcomes.remove(invocation_id))
}

/// The agent a live run spawned, by invocation ID.
#[derive(Serialize, Deserialize, Default)]
struct Agents {
    agents: BTreeMap<String, Spawned>,
}

/// `agent` as the one `invocation_id`'s run spawned, or no agent for it.
pub async fn record_agent(
    data_dir: &Path,
    invocation_id: &str,
    agent: Option<Spawned>,
) -> Result<(), Error> {
    update(data_dir, AGENTS_FILE_NAME, |agents: &mut Agents| {
        match agent {
            Some(agent) => agents.agents.insert(invocation_id.to_string(), agent),
            None => agents.agents.remove(invocation_id),
        };
    })
    .await
}

pub async fn recorded_agent(
    data_dir: &Path,
    invocation_id: &str,
) -> Result<Option<Spawned>, Error> {
    let mut agents: Agents = read(&data_dir.join(AGENTS_FILE_NAME)).await?;
    Ok(agents.agents.remove(invocation_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    async fn load(json: &str) -> Result<Option<TaskSettings>, Error> {
        let dir = TempDir::new()?;
        tokio::fs::write(dir.path().join(FILE_NAME), json).await?;
        TaskSettings::load(dir.path()).await
    }

    #[tokio::test]
    async fn tasks_are_off_without_the_file() -> Result<(), Error> {
        let dir = TempDir::new()?;
        assert_eq!(TaskSettings::load(dir.path()).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn tasks_are_off_with_no_blessed_bucket() -> Result<(), Error> {
        assert_eq!(load(r#"{"version":1,"blessed_buckets":[]}"#).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn the_proposals_file_turns_tasks_on() -> Result<(), Error> {
        let settings = load(
            r#"{"version":1,"blessed_buckets":["s3://quilt-dev"],
                "agents":{"kiro":{"command":["/Applications/Kiro CLI.app/Contents/MacOS/kiro-cli-chat","acp","--agent-engine=v3"],"trust_all":true},
                          "other":{"command":["other-agent"]}}}"#,
        )
        .await?
        .expect("on");
        assert!(settings.blesses("quilt-dev"));
        assert!(!settings.blesses("quilt-prod"));
        let kiro = &settings.agents["kiro"];
        assert_eq!(kiro.command[1..], ["acp", "--agent-engine=v3"]);
        assert!(kiro.trust_all);
        assert!(
            !settings.agents["other"].trust_all,
            "trust_all is off unless set"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_agents_time_limits_are_its_own_or_the_defaults() -> Result<(), Error> {
        let settings = load(
            r#"{"blessed_buckets":["quilt-dev"],
                "agents":{"slow":{"command":["slow"],"startup_timeout_secs":300,"turn_timeout_secs":7200},
                          "plain":{"command":["plain"]}}}"#,
        )
        .await?
        .expect("on");
        let slow = &settings.agents["slow"];
        assert_eq!(slow.startup_timeout(), Duration::from_mins(5));
        assert_eq!(slow.turn_timeout(), Duration::from_hours(2));
        let plain = &settings.agents["plain"];
        assert_eq!(plain.startup_timeout(), Duration::from_mins(2));
        assert_eq!(plain.turn_timeout(), Duration::from_mins(30));
        Ok(())
    }

    #[tokio::test]
    async fn an_invocation_is_remembered_across_reads() -> Result<(), Error> {
        let dir = TempDir::new()?;
        assert!(own_invocations(dir.path()).await?.is_empty());
        remember_invocation(dir.path(), "a").await?;
        remember_invocation(dir.path(), "b").await?;
        assert_eq!(
            own_invocations(dir.path()).await?,
            BTreeSet::from(["a".to_string(), "b".to_string()])
        );
        Ok(())
    }
}

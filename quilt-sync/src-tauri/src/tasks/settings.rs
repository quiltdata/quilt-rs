use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;

const FILE_NAME: &str = "task_settings.json";
const INVOCATIONS_FILE_NAME: &str = "task_invocations.json";

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

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentSettings {
    /// The program and its arguments, run with the package's folder as cwd.
    pub command: Vec<String>,
    /// Approve every permission request the agent makes.
    #[serde(default)]
    pub trust_all: bool,
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

fn invocations_path(data_dir: &Path) -> PathBuf {
    data_dir.join(INVOCATIONS_FILE_NAME)
}

pub async fn own_invocations(data_dir: &Path) -> Result<BTreeSet<String>, Error> {
    match tokio::fs::read(invocations_path(data_dir)).await {
        Ok(bytes) => Ok(serde_json::from_slice::<Invocations>(&bytes)?.invocation_ids),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(err) => Err(Error::from(err)),
    }
}

pub async fn remember_invocation(data_dir: &Path, invocation_id: &str) -> Result<(), Error> {
    let mut invocations = Invocations {
        invocation_ids: own_invocations(data_dir).await?,
    };
    invocations.invocation_ids.insert(invocation_id.to_string());
    tokio::fs::create_dir_all(data_dir).await?;
    let path = invocations_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(&invocations)?).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
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

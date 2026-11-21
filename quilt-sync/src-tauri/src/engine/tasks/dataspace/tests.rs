use std::path::Path;

use quilt_rs::dataspace::METADATA_FILE;
use quilt_rs::dataspace::install::Install;
use quilt_rs::io::remote::mocks::MockRemote;

use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::URI;
use crate::engine::dataspace::tests::World;
use crate::engine::dataspace::tests::raw;

/// A task that asks this desktop's `claude` agent to run `TASK.md`.
const TASK: &str = r#"
[metadata.package.agent_exec]
document = "TASK.md"

[metadata.package.agent_exec.agent]
id = "claude"

[metadata.package.agent_exec.claim]
status = "unclaimed"
"#;

/// Task settings in `data_dir` blessing bucket `b`, with a `claude` agent.
async fn tasks_on(data_dir: &Path) -> Res {
    let settings: TaskSettings = serde_json::from_value(serde_json::json!({
        "blessed_buckets": ["b"],
        "agents": { "claude": { "command": ["true"] } },
    }))?;
    settings.save(data_dir).await?;
    Ok(())
}

/// An agent that writes `out.md` in its folder and, if the task still
/// stands once it is re-read, ends its turn.
fn writing_agent() -> RunAgent {
    Arc::new(|run: AgentRun| {
        Box::pin(async move {
            std::fs::write(run.folder.join("out.md"), "the agent's work").expect("out.md");
            if let Err(outcome) = run.recheck.await {
                return outcome;
            }
            RunOutcome::Done {
                session_id: "s1".to_string(),
                stop_reason: "end_turn".to_string(),
            }
        })
    })
}

/// An agent that writes `out.md`, then never ends, as one the app's exit
/// cuts short.
fn stalled_agent() -> RunAgent {
    Arc::new(|run: AgentRun| {
        Box::pin(async move {
            std::fs::write(run.folder.join("out.md"), "the agent's work").expect("out.md");
            std::future::pending().await
        })
    })
}

/// An agent no test expects to start.
fn no_agent() -> RunAgent {
    Arc::new(|_| Box::pin(async { panic!("no run was expected") }))
}

/// `team/raw` published here with a task, and installed and filled there,
/// on the same bucket, where tasks are on.
struct Task {
    world: World,
    ds: Dataspace<MockRemote>,
    there: World,
    theirs: Arc<Dataspace<MockRemote>>,
}

impl Task {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let world = World::new()?;
        let ds = world.published().await?;
        let there = world.elsewhere()?;
        tasks_on(&there.dir.path().join("app")).await?;
        let theirs = Arc::new(there.open().await?);
        theirs.install(&Install::new(URI)).await?;
        theirs.sync("filled").await?;
        Ok(Self {
            world,
            ds,
            there,
            theirs,
        })
    }

    fn data_dir(&self) -> PathBuf {
        self.there.dir.path().join("app")
    }

    /// Publishes the task here.
    async fn ask(&self) -> Res {
        self.world.write("TASK.md", "Do the thing.")?;
        self.world.write(METADATA_FILE, TASK)?;
        self.ds.sync("task").await?;
        Ok(())
    }

    /// The package's metadata at `latest`, read here.
    async fn latest(
        &self,
    ) -> Result<(Value, quilt_rs::manifest::Manifest), Box<dyn std::error::Error>> {
        let status = self.ds.entry_status(&raw(), true).await?;
        let latest = status.hashes.latest.ok_or("no latest")?;
        let entry = self.ds.entry(&raw()).await?;
        let manifest = self.ds.revision_manifest(&entry, &latest).await?;
        let meta = manifest.header.user_meta.clone().unwrap_or_default();
        Ok((meta, manifest))
    }

    /// Waits until `latest`'s claim has status `status`.
    async fn until_latest_claim(
        &self,
        status: &str,
    ) -> Result<(Value, quilt_rs::manifest::Manifest), Box<dyn std::error::Error>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let (meta, manifest) = self.latest().await?;
            if meta["agent_exec"]["claim"]["status"] == status {
                return Ok((meta, manifest));
            }
            if std::time::Instant::now() > deadline {
                return Err(format!("latest's claim never became {status}: {meta}").into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

fn lists(manifest: &quilt_rs::manifest::Manifest, key: &str) -> bool {
    super::super::lists(manifest, Path::new(key))
}

/// A task a sync reads is found from the `post-read` hook, claimed by a
/// revision of its own, run in the folder, and its outcome published to
/// the bucket with what the agent wrote.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_found_on_post_read_is_claimed_run_and_published() -> Res {
    let task = Task::new().await?;
    let tasks = DataspaceTasks::new(Arc::clone(&task.theirs), task.data_dir(), writing_agent());
    let _hook = tasks.register();

    task.ask().await?;
    task.theirs.sync("read").await?;
    let (meta, manifest) = task.until_latest_claim("done").await?;

    let claim = &meta["agent_exec"]["claim"];
    let own = settings::own_invocations(&task.data_dir()).await?;
    assert!(
        claim["invocation_id"]
            .as_str()
            .is_some_and(|id| own.contains(id)),
        "{claim}"
    );
    assert_eq!(claim["changes"]["added"], 1, "{claim}");
    assert!(lists(&manifest, "out.md"), "the agent's work is published");
    assert_eq!(manifest.header.message(), Some("Task TASK.md done"));
    Ok(())
}

/// A run the app's exit cut short is found on the next start, as this
/// desktop's own claim with no run, and the folder, with what the agent
/// left, published as `failed`.
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_task_is_recovered_on_the_next_start() -> Res {
    let task = Task::new().await?;
    let first = DataspaceTasks::new(Arc::clone(&task.theirs), task.data_dir(), stalled_agent());
    let hook = first.register();
    task.ask().await?;
    task.theirs.sync("read").await?;
    task.until_latest_claim("claimed").await?;
    let left = task.there.folder().join("out.md");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !left.exists() {
        assert!(std::time::Instant::now() < deadline, "the agent never ran");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // The app exits mid-run.
    first.shutdown();
    drop(hook);
    drop(first);
    // Let the aborted run drop the package it held.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let next = DataspaceTasks::new(Arc::clone(&task.theirs), task.data_dir(), no_agent());
    next.scan().await;
    let (meta, manifest) = task.until_latest_claim("failed").await?;

    assert_eq!(
        meta["agent_exec"]["claim"]["reason"], "interrupted by restart",
        "{meta}"
    );
    assert!(
        lists(&manifest, "out.md"),
        "what the agent left is published"
    );
    assert_eq!(
        manifest.header.message(),
        Some("Task failed: interrupted by restart")
    );
    Ok(())
}

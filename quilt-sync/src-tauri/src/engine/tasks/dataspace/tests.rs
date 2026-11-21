use quilt_rs::dataspace::METADATA_FILE;
use quilt_rs::dataspace::install::Install;

use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::URI;
use crate::engine::dataspace::tests::World;

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

/// A revision a sync reads fires the `post-read` hook, and the dispatch
/// finds the task it asks for; a sync that reads nothing new dispatches
/// nothing.
#[tokio::test(flavor = "multi_thread")]
async fn agent_exec_is_dispatched_on_post_read() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    let there = world.elsewhere()?;
    let data_dir = there.dir.path().join("app");
    tasks_on(&data_dir).await?;
    let theirs = Arc::new(there.open().await?);
    theirs.install(&Install::new(URI)).await?;
    theirs.sync("filled").await?;
    let (sent, mut found) = tokio::sync::mpsc::unbounded_channel();
    let _registration = register(
        &theirs,
        data_dir,
        Arc::new(move |call: &HookCall, verdict: Verdict| {
            let _ = sent.send((call.revision.clone(), verdict));
        }),
    );

    theirs.sync("idle").await?;
    world.write("TASK.md", "Do the thing.")?;
    world.write(METADATA_FILE, TASK)?;
    let published = ds.sync("task").await?;
    theirs.sync("read").await?;

    let (revision, verdict) =
        tokio::time::timeout(std::time::Duration::from_secs(10), found.recv())
            .await?
            .ok_or("the dispatch ended")?;
    assert_eq!(
        verdict,
        Verdict::Claim {
            document: "TASK.md".to_string(),
            agent: "claude".to_string(),
        }
    );
    let entry = theirs
        .entry(&crate::engine::dataspace::tests::raw())
        .await?;
    let lock = quilt_rs::dataspace::lock::Lock::read(theirs.storage(), &theirs.contract().await?)
        .await?
        .ok_or("a lock")?;
    assert_eq!(
        lock.packages[&entry.key()].base.as_deref(),
        Some(revision.as_str())
    );
    assert!(!published.entries.is_empty());
    assert!(
        found.try_recv().is_err(),
        "the idle sync dispatched nothing"
    );
    Ok(())
}

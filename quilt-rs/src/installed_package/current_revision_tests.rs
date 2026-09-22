use chrono::Utc;
use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::io::storage::Storage;
use crate::local_domain::LocalDomain;

#[test(tokio::test)]
async fn current_revision_reads_only_the_selected_installed_manifest() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    let namespace: Namespace = ("demo", "sales").into();
    let before = Utc::now();
    let package = domain
        .create_package(namespace.clone(), None, Some("initial import".to_string()))
        .await?;
    let expected_hash = package
        .lineage()
        .await?
        .current_hash()
        .expect("a created package has a current revision")
        .to_string();

    // A current-revision read must not enumerate and parse history. Keeping a
    // corrupt sibling here makes that contract observable in the test.
    package
        .storage
        .write_byte_stream(
            package
                .paths
                .installed_manifest(&namespace, "damaged-old-revision"),
            b"not a manifest".to_vec().into(),
        )
        .await?;

    let revision = package
        .current_revision()
        .await?
        .expect("a created package has a current revision");
    let after = Utc::now();

    assert_eq!(revision.hash, expected_hash);
    assert_eq!(revision.message.as_deref(), Some("initial import"));
    assert!(revision.obtained >= before);
    assert!(revision.obtained <= after);

    Ok(())
}

#[test(tokio::test)]
async fn current_revision_prefers_a_pending_commit_over_the_remote_hash() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    let namespace: Namespace = ("demo", "pending").into();
    let package = domain
        .create_package(namespace.clone(), None, Some("local work".to_string()))
        .await?;
    let pending_hash = package
        .lineage()
        .await?
        .commit
        .expect("a locally created package has a pending commit")
        .hash;
    let remote_hash = "published-revision".to_string();

    package
        .lineage
        .edit(&package.storage, |lineage| {
            lineage.remote_uri = Some(ManifestUri {
                origin: None,
                bucket: "published-bucket".to_string(),
                namespace: namespace.clone(),
                hash: remote_hash.clone(),
            });
        })
        .await?;

    let revision = package
        .current_revision()
        .await?
        .expect("the pending commit is the current revision");

    assert_ne!(pending_hash, remote_hash);
    assert_eq!(revision.hash, pending_hash);
    assert_eq!(revision.message.as_deref(), Some("local work"));

    Ok(())
}

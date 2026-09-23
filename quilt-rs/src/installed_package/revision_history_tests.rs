use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::local_domain::LocalDomain;

/// The trigger's count reads the directory and parses nothing: a damaged
/// historical manifest is still a revision this copy holds, and must not
/// fail the page read that carries the count.
#[test(tokio::test)]
async fn the_count_is_the_manifest_files_and_parses_none_of_them() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    let namespace: Namespace = ("demo", "sales").into();
    let package = domain
        .create_package(namespace.clone(), None, Some("initial import".to_string()))
        .await?;

    package
        .storage
        .write_byte_stream(
            package
                .paths
                .installed_manifest(&namespace, "damaged-old-revision"),
            b"not a manifest".to_vec().into(),
        )
        .await?;
    // A directory beside the manifests is not a revision.
    package
        .storage
        .create_dir_all(
            package
                .paths
                .installed_manifests_dir(&namespace)
                .join("not-a-revision"),
        )
        .await?;

    assert_eq!(package.revision_count().await?, 2);
    Ok(())
}

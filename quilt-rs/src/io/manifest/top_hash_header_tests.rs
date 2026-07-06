//! Top-hash pinning tests: manifest-header variants (message/meta/workflow).
//!
//! Every constant in `fixtures::top_hash` is an interop anchor shared with
//! the Python client — these tests must never be updated to new hashes.

use super::*;

use test_log::test;

use crate::fixtures::top_hash;
use crate::io::storage::LocalStorage;
use crate::io::storage::mocks::MockStorage;

#[test(tokio::test)]
async fn test_empty_manifest_header_empty() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        Manifest::default().header,
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_EMPTY_TOP_HASH));
    assert_eq!(top_hash, top_hash::EMPTY_EMPTY_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::EMPTY_EMPTY_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_none() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            user_meta: None,
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_NONE_TOP_HASH));
    assert_eq!(top_hash, top_hash::EMPTY_NONE_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NONE_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::EMPTY_NONE_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_null() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_NULL_TOP_HASH));
    assert_eq!(top_hash, top_hash::EMPTY_NULL_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NULL_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::EMPTY_NULL_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_null_empty() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: None,
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::NULL_EMPTY_TOP_HASH));
    assert_eq!(top_hash, top_hash::NULL_EMPTY_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::NULL_EMPTY_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::NULL_EMPTY_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_null_none() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: None,
            user_meta: None,
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::NULL_NONE_TOP_HASH));
    assert_eq!(top_hash, top_hash::NULL_NONE_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::NULL_NONE_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::NULL_NONE_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_null_null() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: None,
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::NULL_NULL_TOP_HASH));
    assert_eq!(top_hash, top_hash::NULL_NULL_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::NULL_NULL_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::NULL_NULL_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_empty() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_EMPTY_TOP_HASH));
    assert_eq!(top_hash, top_hash::INITIAL_EMPTY_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_EMPTY_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_EMPTY_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_none() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_NONE_TOP_HASH));
    assert_eq!(top_hash, top_hash::INITIAL_NONE_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_NONE_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_NONE_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_null() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_NULL_TOP_HASH));
    assert_eq!(top_hash, top_hash::INITIAL_NULL_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_NULL_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_NULL_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_meta() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_META_TOP_HASH));
    assert_eq!(top_hash, top_hash::INITIAL_META_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_META_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_META_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_complex_meta() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"author": "user", "timestamp": "2024-01-01"})),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_COMPLEX_META_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);

    // Create manifest from text content and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_COMPLEX_META_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_large_meta() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let large_meta = serde_json::json!({
        "author": "user",
        "timestamp": "2024-01-01T10:30:00Z",
        "description": "This is a comprehensive test with larger metadata",
        "tags": ["test", "manifest", "quilt"],
        "version": 1,
        "nested": {
            "key1": "value1",
            "key2": 42,
            "key3": true
        }
    });
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(large_meta.clone()),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_LARGE_META_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_LARGE_META_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_empty_simple_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_empty_complex_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_empty_simple_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_empty_complex_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_none_simple_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some(String::new()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_empty_null_simple_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::Value::Null),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_meta_simple_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_empty_manifest_header_initial_none_complex_workflow() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let (dest_path, top_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        },
        tokio_stream::empty(),
    )
    .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH);

    // Create manifest from fixture file and verify top_hash matches
    let fixture_path = top_hash::load_fixture(top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash,
        top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH
    );
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

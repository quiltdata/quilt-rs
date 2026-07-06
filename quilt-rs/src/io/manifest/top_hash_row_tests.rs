//! Top-hash pinning tests: manifests with entry rows and mixed hash types.
//!
//! Every constant in `fixtures::top_hash` is an interop anchor shared with
//! the Python client — these tests must never be updated to new hashes.

use super::*;

use test_log::test;

use crate::checksum::Crc64Hash;
use crate::checksum::Sha256ChunkedHash;
use crate::checksum::Sha256Hash;
use crate::fixtures::objects;
use crate::fixtures::top_hash;
use crate::io::storage::LocalStorage;
use crate::io::storage::mocks::MockStorage;

#[test(tokio::test)]
async fn test_single_row_manifest() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let header = ManifestHeader::default();

    let manifest_row = ManifestRow {
        logical_key: PathBuf::from("data.txt"),
        physical_key: "s3://bucket/data.txt".to_string(),
        hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
        size: 16,
        meta: Some(serde_json::json!({"type": "text"})),
    };

    let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(manifest_row)])]);
    let (dest_path, top_hash) =
        build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
            .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::SINGLE_ROW_TOP_HASH));
    assert_eq!(top_hash, top_hash::SINGLE_ROW_TOP_HASH);

    // Verify using Manifest::from_path with the fixture file
    let fixture_path = top_hash::load_fixture(top_hash::SINGLE_ROW_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::SINGLE_ROW_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_mixed_hash_types_manifest() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let header = ManifestHeader::default();

    let row1 = ManifestRow {
        logical_key: PathBuf::from("file1.txt"),
        physical_key: "s3://bucket/file1.txt".to_string(),
        hash: Sha256Hash::try_from(
            "7465737464617461000000000000000000000000000000000000000000000000",
        )?
        .into(),
        size: 8,
        meta: None,
    };

    let row2 = ManifestRow {
        logical_key: PathBuf::from("file2.txt"),
        physical_key: "s3://bucket/file2.txt".to_string(),
        hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
        size: 16,
        meta: None,
    };

    let row3 = ManifestRow {
        logical_key: PathBuf::from("file3.txt"),
        physical_key: "s3://bucket/file3.txt".to_string(),
        hash: Crc64Hash::try_from("dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA")?.into(),
        size: 32,
        meta: None,
    };

    let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(row1), Ok(row2), Ok(row3)])]);
    let (dest_path, top_hash) =
        build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
            .await?;
    assert_eq!(
        dest_path,
        dest_dir.join(top_hash::MIXED_HASH_TYPES_TOP_HASH)
    );
    assert_eq!(top_hash, top_hash::MIXED_HASH_TYPES_TOP_HASH);

    // Verify using Manifest::from_path with the fixture file
    let fixture_path = top_hash::load_fixture(top_hash::MIXED_HASH_TYPES_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash_from_reader) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(
        calculated_hash_from_reader,
        top_hash::MIXED_HASH_TYPES_TOP_HASH
    );
    assert_eq!(calculated_hash_from_reader, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_multiple_rows_manifest() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let header = ManifestHeader::default();

    let row1 = ManifestRow {
        logical_key: PathBuf::from("config.json"),
        physical_key: "s3://bucket/config.json".to_string(),
        hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
        size: 0,
        meta: Some(serde_json::json!({"format": "json"})),
    };

    let row2 = ManifestRow {
        logical_key: PathBuf::from("data/file.csv"),
        physical_key: "s3://bucket/data/file.csv".to_string(),
        hash: Sha256ChunkedHash::try_from(objects::EQUAL_TO_8MB_HASH_B64)?.into(),
        size: 8_388_608,
        meta: Some(serde_json::Value::Null),
    };

    let row3 = ManifestRow {
        logical_key: PathBuf::from("images/photo.jpg"),
        physical_key: "s3://bucket/images/photo.jpg".to_string(),
        hash: Sha256ChunkedHash::try_from(objects::MORE_THAN_8MB_HASH_B64)?.into(),
        size: 18_874_368,
        meta: Some(serde_json::json!({"width": 1920, "height": 1080})),
    };

    let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(row1), Ok(row2), Ok(row3)])]);
    let (dest_path, top_hash) =
        build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
            .await?;
    assert_eq!(dest_path, dest_dir.join(top_hash::MULTIPLE_ROWS_TOP_HASH));
    assert_eq!(top_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);

    // Verify using Manifest::from_path with the fixture file
    let fixture_path = top_hash::load_fixture(top_hash::MULTIPLE_ROWS_TOP_HASH)?;
    let local_storage = LocalStorage::default();
    let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
    let (_, calculated_hash) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest.header.clone(),
        manifest.records_stream().await,
    )
    .await?;

    assert_eq!(calculated_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);
    assert_eq!(calculated_hash, top_hash);

    Ok(())
}

#[test(tokio::test)]
async fn test_hash_normalization_equivalence_manifest() -> Res {
    let storage = MockStorage::default();
    let dest_dir = storage.temp_dir.path();
    let local_storage = LocalStorage::default();

    // Load all three variant fixture files
    let fixture_path1 =
        top_hash::load_equivalent_fixture(top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH, "canonical")?;
    let fixture_path2 = top_hash::load_equivalent_fixture(
        top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH,
        "meta-null-key-order",
    )?;
    let fixture_path3 = top_hash::load_equivalent_fixture(
        top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH,
        "field-order-missing-meta",
    )?;

    // Load manifests from fixture files
    let manifest1 = Manifest::from_path(&local_storage, &fixture_path1).await?;
    let manifest2 = Manifest::from_path(&local_storage, &fixture_path2).await?;
    let manifest3 = Manifest::from_path(&local_storage, &fixture_path3).await?;

    // Calculate hashes for all three variants
    let (_, calculated_hash1) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest1.header.clone(),
        manifest1.records_stream().await,
    )
    .await?;

    let (_, calculated_hash2) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest2.header.clone(),
        manifest2.records_stream().await,
    )
    .await?;

    let (_, calculated_hash3) = build_manifest_from_rows_stream(
        &storage,
        dest_dir.to_path_buf(),
        manifest3.header.clone(),
        manifest3.records_stream().await,
    )
    .await?;

    // All three variants should produce the same hash despite different representations
    assert_eq!(
        calculated_hash1, calculated_hash2,
        "Canonical and meta-null-key-order variants should normalize to same hash"
    );
    assert_eq!(
        calculated_hash1, calculated_hash3,
        "Canonical and field-order-missing-meta variants should normalize to same hash"
    );
    assert_eq!(
        calculated_hash2, calculated_hash3,
        "All meta empty representations should normalize to same hash"
    );

    // Test that the normalized hash matches our expected constant
    assert_eq!(calculated_hash1, top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH);

    Ok(())
}

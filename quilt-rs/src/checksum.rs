//! Domain helpers that compute object hashes for manifest rows.
//!
//! The hash value types themselves live in [`crate::object_hash`]; this module
//! layers the storage- and host-config-aware orchestration on top: hashing a
//! working file with the algorithm a host expects, and refreshing/verifying a
//! row's hash against the file on disk.

use std::path::Path;
use std::path::PathBuf;

use crate::Res;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::storage::Storage;
use crate::manifest::ManifestRow;
use crate::object_hash::Crc64Hash;
use crate::object_hash::Hash;
use crate::object_hash::ObjectHash;
use crate::object_hash::Sha256ChunkedHash;
use crate::object_hash::Sha256Hash;

/// Refresh hash for a file using the same algorithm as the reference row
/// Returns None if hash hasn't changed, Some(ManifestRow) if it has changed
pub async fn refresh_hash(
    storage: &impl Storage,
    path: &PathBuf,
    row: ManifestRow,
) -> Res<Option<ManifestRow>> {
    let file = storage.open_file(path).await?;
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();

    let computed_hash = match &row.hash {
        ObjectHash::Crc64(_) => Crc64Hash::from_reader(file, size).await?.into(),
        ObjectHash::Sha256(_) => Sha256Hash::from_reader(file, size).await?.into(),
        ObjectHash::Sha256Chunked(_) => Sha256ChunkedHash::from_reader(file, size).await?.into(),
    };

    Ok((computed_hash != row.hash).then(|| ManifestRow {
        hash: computed_hash,
        size,
        ..row
    }))
}

/// Calculate hash for a file using the algorithm specified by host config
pub async fn calculate_hash(
    storage: &impl Storage,
    path: &Path,
    logical_key: &Path,
    host_config: &HostConfig,
) -> Res<ManifestRow> {
    let file = storage.open_file(path).await?;
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();

    let hash = match host_config.checksums {
        HostChecksums::Crc64 => Crc64Hash::from_reader(file, size).await?.into(),
        HostChecksums::Sha256Chunked => Sha256ChunkedHash::from_reader(file, size).await?.into(),
    };

    Ok(ManifestRow {
        logical_key: logical_key.to_path_buf(),
        physical_key: format!("file://{}", path.display()),
        size,
        hash,
        ..ManifestRow::default()
    })
}

/// Verify hash for a file and optionally recalculate with host's preferred algorithm
/// Returns None if hash hasn't changed, Some(ManifestRow) if it has changed
pub async fn verify_hash(
    storage: &impl Storage,
    path: &PathBuf,
    row: ManifestRow,
    host_config: &HostConfig,
) -> Res<Option<ManifestRow>> {
    if let Some(modified) = refresh_hash(storage, path, row).await? {
        // File has changed, check if we need to recalculate with host's preferred algorithm
        if modified.hash.algorithm() == host_config.checksums.algorithm_code() {
            // Already using the correct algorithm, no need to recalculate
            Ok(Some(modified))
        } else {
            // Need to recalculate with host's preferred algorithm
            let calculated_row =
                calculate_hash(storage, path, &modified.logical_key, host_config).await?;
            Ok(Some(calculated_row))
        }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use std::path::Path;

    use aws_sdk_s3::primitives::ByteStream;
    use multihash::Multihash;

    use crate::Res;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::Storage;
    use crate::io::storage::StorageExt;
    use crate::io::storage::mocks::MockStorage;
    use crate::object_hash::MULTIHASH_CRC64_NVME;
    use crate::object_hash::MULTIHASH_SHA256;
    use crate::object_hash::MULTIHASH_SHA256_CHUNKED;

    #[test(tokio::test)]
    async fn test_refresh_hash_unchanged_file() -> Res {
        let storage = MockStorage::default();
        let file_content = b"anything";
        let file_path = Path::new("foo");
        storage
            .write_byte_stream(file_path, ByteStream::from_static(file_content))
            .await?;

        let file = storage.open_file(file_path).await?;
        let hash: Multihash<256> = Sha256Hash::from_reader(file, file_content.len() as u64)
            .await?
            .into();

        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("bar"),
            hash: hash.try_into()?,
            size: file_content.len() as u64,
            ..ManifestRow::default()
        };
        let result = refresh_hash(&storage, &file_path.to_path_buf(), manifest_row).await?;
        assert!(result.is_none(), "Unchanged file should return None");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_refresh_hash_changed_file() -> Res {
        let storage = MockStorage::default();
        let file_content = b"anything";
        let file_path = Path::new("foo");
        storage
            .write_byte_stream(file_path, ByteStream::from_static(file_content))
            .await?;

        // Calculate the actual hash first
        let file = storage.open_file(file_path).await?;
        let hash: Multihash<256> = Sha256Hash::from_reader(file, file_content.len() as u64)
            .await?
            .into();

        // Test with wrong hash - should return updated ManifestRow
        let wrong_hash = Multihash::wrap(MULTIHASH_SHA256, b"wrong_hash_data")?;
        let test_manifest_row = ManifestRow {
            logical_key: PathBuf::from("bar"),
            hash: wrong_hash.try_into()?,
            size: 999, // Wrong size to test that refresh_hash updates it
            ..ManifestRow::default()
        };
        let result = refresh_hash(&storage, &file_path.to_path_buf(), test_manifest_row).await?;
        let refreshed_row = result.expect("Changed hash should return Some");
        assert_eq!(refreshed_row.hash, hash.try_into()?);
        assert_eq!(refreshed_row.size, file_content.len() as u64);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_refresh_hash_unknown_algorithm() -> Res {
        let storage = MockStorage::default();
        let file_content = b"anything";
        let file_path = Path::new("foo");
        storage
            .write_byte_stream(file_path, ByteStream::from_static(file_content))
            .await?;

        // Since ObjectHash now validates hash types, we cannot create invalid hashes
        // This test is no longer relevant as the type system prevents invalid hash codes
        // Let's test a valid case instead - using a valid hash should work
        let valid_hash = Crc64Hash::default().into();
        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("bar"),
            hash: valid_hash,
            size: file_content.len() as u64,
            ..ManifestRow::default()
        };

        let result = refresh_hash(&storage, &file_path.to_path_buf(), manifest_row).await;
        // With valid hash, this should work (though it might return Some due to different file content)
        assert!(result.is_ok());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_calculate_hash_crc64() -> Res {
        let storage = LocalStorage::default();

        let file_path = Path::new("fixtures/user-settings.mkfg");
        let host_config = HostConfig::default_crc64();
        let logical_key = PathBuf::from("foo");

        let row = calculate_hash(&storage, file_path, &logical_key, &host_config).await?;

        assert_eq!(row.hash.algorithm(), MULTIHASH_CRC64_NVME);
        assert_eq!(row.logical_key, logical_key);

        assert_eq!(row.size, storage.read_bytes(file_path).await?.len() as u64);
        assert_eq!(row.hash.to_string(), "LZmmpqbBItw=");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_calculate_hash_sha256_chunked() -> Res {
        let storage = MockStorage::default();

        let host_config = HostConfig::default_sha256_chunked();

        let file_content = crate::fixtures::objects::less_than_8mb();
        let file_path = Path::new("foo");
        storage
            .write_byte_stream(file_path, ByteStream::from_static(file_content))
            .await?;

        let logical_key = PathBuf::from("bar");

        let row = calculate_hash(&storage, file_path, &logical_key, &host_config).await?;

        assert_eq!(row.hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(row.logical_key, logical_key);
        assert_eq!(row.size, file_content.len() as u64);
        assert_eq!(
            row.hash.to_string(),
            crate::fixtures::objects::LESS_THAN_8MB_HASH_B64
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_verify_hash_crc64() -> Res {
        let storage = MockStorage::default();
        let local_storage = LocalStorage::default();

        // Create file with initial content
        let test_path = Path::new("foo");
        let initial_content = b"lorem ipsum";
        storage
            .write_byte_stream(test_path, ByteStream::from_static(initial_content))
            .await?;

        let sha256_host_config = HostConfig::default_sha256_chunked();
        let crc64_host_config = HostConfig::default_crc64();
        let logical_key = PathBuf::from("bar");

        let manifest_row =
            calculate_hash(&storage, test_path, &logical_key, &sha256_host_config).await?;

        assert!(
            verify_hash(
                &storage,
                &test_path.to_path_buf(),
                manifest_row.clone(),
                &crc64_host_config,
            )
            .await?
            .is_none(),
            "Unchanged file should return None, even when algorithms don't match"
        );

        let fixture_path = Path::new("fixtures/user-settings.mkfg");
        let fixture_content = local_storage.read_byte_stream(fixture_path).await?;
        storage
            .write_byte_stream(test_path, fixture_content)
            .await?;

        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            manifest_row,
            &crc64_host_config,
        )
        .await?;
        assert!(result.is_some(), "Modified file should return Some");

        let modified_row = result.unwrap();
        assert_eq!(modified_row.hash.algorithm(), MULTIHASH_CRC64_NVME);

        assert_eq!(modified_row.hash.to_string(), "LZmmpqbBItw=");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_verify_hash_sha256_chunked() -> Res {
        let storage = MockStorage::default();

        // Create file with initial content
        let test_path = Path::new("foo");
        let initial_content = b"lorem ipsum";
        storage
            .write_byte_stream(test_path, ByteStream::from_static(initial_content))
            .await?;

        let sha256_host_config = HostConfig::default_sha256_chunked();
        let crc64_host_config = HostConfig::default_crc64();
        let logical_key = PathBuf::from("bar");

        let manifest_row =
            calculate_hash(&storage, test_path, &logical_key, &crc64_host_config).await?;

        assert!(
            verify_hash(
                &storage,
                &test_path.to_path_buf(),
                manifest_row.clone(),
                &sha256_host_config,
            )
            .await?
            .is_none(),
            "Unchanged file should return None, even when algorithms don't match"
        );

        // Now rewrite file with less_than_8mb fixture content
        let fixture_content = crate::fixtures::objects::less_than_8mb();
        storage
            .write_byte_stream(test_path, ByteStream::from_static(fixture_content))
            .await?;

        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            manifest_row,
            &sha256_host_config,
        )
        .await?;
        assert!(result.is_some(), "Modified file should return Some");

        let modified_row = result.unwrap();
        assert_eq!(modified_row.hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(modified_row.size, fixture_content.len() as u64);

        assert_eq!(
            modified_row.hash.to_string(),
            crate::fixtures::objects::LESS_THAN_8MB_HASH_B64
        );

        Ok(())
    }
}

use std::path::PathBuf;

use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use aws_smithy_types::base64;
use multihash::Multihash;

use crate::Error;
use crate::Res;

fn local_uri(key: &str) -> Res<PathBuf> {
    Ok(std::env::current_dir()?.join(key))
}

pub mod manifest {
    use super::local_uri;

    use std::path::PathBuf;

    use crate::Res;

    const TEST_LOCAL_CHECKSUMMED: &str = "fixtures/checksummed.jsonl";

    pub const CHECKSUMMED_HASH: &str =
        "87c4c6caa2eaeb346ea8c3a5c3650a54b9ae93ed5229db67eac7449fc198f0c5";

    pub fn checksummed() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_CHECKSUMMED)
    }
}

pub mod manifest_empty {
    /// Manifest header: {"message":"","user_meta":{},"version":"v0"}
    pub const EMPTY_EMPTY_TOP_HASH: &str =
        "0929824c58e90a6d2cc3ad2c7bdc66e34f43e8ed7063a6b48595a3834dd3ec99";

    /// Manifest header: {"message":"","version":"v0"}
    pub const EMPTY_NONE_TOP_HASH: &str =
        "770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";

    /// Manifest header: {"message":"","user_meta":null,"version":"v0"}
    pub const EMPTY_NULL_TOP_HASH: &str =
        "39ee9fb46019db2d8373c991d7881ba90bbb6a6c65417e108c295363794dec3c";

    /// Manifest header: {"message":null,"user_meta":{},"version":"v0"}
    pub const NULL_EMPTY_TOP_HASH: &str =
        "1a4cae60caad99aaf073c4292adfa510897c31c1d6adb44662925b9b143edbbf";

    /// Manifest header: {"message":null,"version":"v0"}
    pub const NULL_NONE_TOP_HASH: &str =
        "2a5a67156ca9238c14d12042db51c5b52260fdd5511b61ea89b58929d6e1769b";

    /// Manifest header: {"message":null,"user_meta":null,"version":"v0"}
    pub const NULL_NULL_TOP_HASH: &str =
        "fb53faf10edc3c9cc234f362c1a57d702213e869a20b887d9f6ed5439c936513";

    // INITIAL_* series: message="Initial"

    /// Manifest header: {"message":"Initial","user_meta":{},"version":"v0"}
    pub const INITIAL_EMPTY_TOP_HASH: &str =
        "7ee06a4f805b84b2f43ddad3f68bcbf7aafad2412da86e2e912cdfe139eea7f0";

    /// Manifest header: {"message":"Initial","version":"v0"}
    pub const INITIAL_NONE_TOP_HASH: &str =
        "5c28e0c17258bff26192df9fd307cbcd72ca9c72777e006282b2508827e76874";

    /// Manifest header: {"message":"Initial","user_meta":null,"version":"v0"}
    pub const INITIAL_NULL_TOP_HASH: &str =
        "82d4864583067f3dba8909f050fb8dc0b8e00a77e7b5e14a53aacb165740c7a4";

    /// Manifest header: {"message":"Initial","user_meta":{"key":"value"},"version":"v0"}
    pub const INITIAL_META_TOP_HASH: &str =
        "0d659c7f1d7a141160a9defc9b1c9ea7bca96d3454af59cbfcf523871e72f47e";

    /// Manifest header: {"message":"Initial","user_meta":{"author":"user","timestamp":"2024-01-01"},"version":"v0"}
    pub const INITIAL_COMPLEX_META_TOP_HASH: &str =
        "9bf52db215ab75c1f75fe4b1b4a782cf822eb463f9f8395e95f3b47627a0e825";

    /// Manifest header: {"message":"Initial","user_meta":{large_object},"version":"v0"}
    pub const INITIAL_LARGE_META_TOP_HASH: &str =
        "df9d3129d62e60c1ea840d9c147e2ba7c94269bb33382026c5b27a20cd1351aa";

    // WORKFLOW series: with workflow field

    /// Manifest header: {"message":"","user_meta":{},"version":"v0","workflow":{"config":"s3://workflow/config","id":null}}
    pub const EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH: &str =
        "77cb48f84c2109fcf9e10fd230497f2a3803427bb6b48c32f2c026e080ee1553";

    /// Manifest header: {"message":"","user_meta":{},"version":"v0","workflow":{"config":"s3://workflow/config","id":"test-workflow","schemas":{"test-schema":"s3://bucket/workflows/test.json"}}}
    pub const EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH: &str =
        "714b1c209a98a7b9239076b94305a7852dc60946c5ba0afac64246ea9958ba08";

    /// Manifest header: {"message":"Initial","user_meta":{},"version":"v0","workflow":{"config":"s3://workflow/config","id":null}}
    pub const INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH: &str =
        "c716c54535bd3c896d0813dafd672430456f68b2d407a6a65a558ccab53f4990";

    /// Manifest header: {"message":"Initial","user_meta":{},"version":"v0","workflow":{"config":"s3://workflow/config","id":"test-workflow","schemas":{"test-schema":"s3://bucket/workflows/test.json"}}}
    pub const INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH: &str =
        "63d5e6aedc10aeca11a4ad133b21ecfbc299476833465f6a302fd27cc08d8ab2";
}

pub fn create_multihash(b64_str: &str) -> Res<Multihash<256>> {
    Ok(Multihash::wrap(
        MULTIHASH_SHA256_CHUNKED,
        &base64::decode(b64_str).map_err(|e| Error::Checksum(e.to_string()))?,
    )?)
}

pub mod manifest_with_objects_all_sizes {
    use std::path::PathBuf;

    use super::create_multihash;
    use super::local_uri;
    use super::objects;

    use crate::manifest::Manifest;
    use crate::manifest::ManifestRow;
    use crate::Res;

    const JSONL: &str = "fixtures/ref-manifest-sizes.jsonl";

    pub const TOP_HASH: &str = "a8287f20eb1e315a08ce08d9488dc1e8c75ba45d4549bb4351a74c92b217c3c0";

    pub fn jsonl_path() -> Res<PathBuf> {
        local_uri(JSONL)
    }

    pub async fn manifest() -> Res<Manifest> {
        let mut manifest = Manifest::default();
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("0mb.bin"),
                size: 0,
                hash: create_multihash(objects::ZERO_HASH_B64)?.try_into()?,
                ..ManifestRow::default()
            })
            .await?;
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("bigger-than-8mb.txt"),
                size: 18874368,
                hash: create_multihash(objects::MORE_THAN_8MB_HASH_B64)?.try_into()?,
                ..ManifestRow::default()
            })
            .await?;
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("equal-to-8mb.txt"),
                size: 8388608,
                hash: create_multihash(objects::EQUAL_TO_8MB_HASH_B64)?.try_into()?,
                ..ManifestRow::default()
            })
            .await?;
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("less-then-8mb.txt"),
                size: 16,
                hash: create_multihash(objects::LESS_THAN_8MB_HASH_B64)?.try_into()?,
                ..ManifestRow::default()
            })
            .await?;
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("one/two two/three three three/READ ME.md"),
                size: 20,
                hash: create_multihash(objects::NESTED_HASH_B64)?.try_into()?,
                ..ManifestRow::default()
            })
            .await?;
        Ok(manifest)
    }
}

pub mod objects {
    pub const ZERO_HASH_B64: &str = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";

    pub const ZERO_HASH_HEX: &str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    pub fn zero_bytes<'a>() -> &'a [u8] {
        &[]
    }

    pub const LESS_THAN_8MB_HASH_B64: &str = "Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA=";

    pub const LESS_THAN_8MB_HASH_HEX: &str =
        "5dbd4f6e325e5a87f8cc3ef3b8773d3c8eec2e2cff6248f882986569912ddf10";

    pub fn less_than_8mb<'a>() -> &'a [u8] {
        "0123456789abcdef".as_bytes()
    }

    pub const EQUAL_TO_8MB_HASH_B64: &str = "7V3rZ3Q/AmAYax2wsQBZbc7N1EMIxlxRyMiMthGRdwg=";

    pub const EQUAL_TO_8MB_HASH_HEX: &str =
        "ed5deb67743f0260186b1db0b100596dcecdd44308c65c51c8c88cb611917708";

    pub fn equal_to_8mb() -> Vec<u8> {
        "12345678".as_bytes().repeat(1024 * 1024)
    }

    pub const MORE_THAN_8MB_HASH_B64: &str = "T+rt/HKRJOiAkEGXKvc+DhCwRcrZiDrFkjKonDT1zgs=";

    pub const MORE_THAN_8MB_HASH_HEX: &str =
        "4feaedfc729124e8809041972af73e0e10b045cad9883ac59232a89c34f5ce0b";

    pub fn more_than_8mb() -> Vec<u8> {
        "1234567890abcdefgh".as_bytes().repeat(1024 * 1024)
    }

    pub const NESTED_HASH_B64: &str = "J6TS3FqxN+VOhVoaoPU5OsYMUsq6652ykBrlW7krP/k=";

    pub fn nested<'a>() -> &'a [u8] {
        "This is the README.".as_bytes()
    }
}

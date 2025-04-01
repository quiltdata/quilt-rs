use std::path::PathBuf;

use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;

use crate::Res;

pub mod sample_file_1 {
    use std::path::PathBuf;

    use multihash::Multihash;

    use crate::checksum::ContentHash;
    use crate::lineage::PathState;
    use crate::manifest::Row;
    use crate::Res;

    // FIXME: remove it
    fn row_hash() -> Res<Multihash<256>> {
        // This is a hash of fixtures/manifest.jsonl file
        ContentHash::SHA256Chunked("4ssEkl5yUwi0LCjnsOl3pJ6ZgtgD8o5a6K9ayFtKDQE=".to_string())
            .try_into()
    }

    // FIXME: remove it
    pub fn path_state() -> Res<PathState> {
        Ok(PathState {
            hash: row_hash()?,
            ..PathState::default()
        })
    }

    // FIXME: remove it
    pub fn row(name: PathBuf) -> Res<Row> {
        Ok(Row {
            name,
            place: "file:///z/x/y".into(),
            hash: row_hash()?,
            ..Row::default()
        })
    }
}

fn local_uri(key: &str) -> Res<PathBuf> {
    Ok(std::env::current_dir()?.join(key))
}

pub mod manifest {
    use super::local_uri;

    use std::path::PathBuf;

    use crate::Res;

    const TEST_LOCAL_PARQUET: &str = "fixtures/manifest.parquet";
    const TEST_LOCAL_PARQUET_CHECKSUMMED: &str = "fixtures/checksummed.parquet";
    const TEST_LOCAL_JSONL: &str = "fixtures/manifest.jsonl";

    pub const JSONL_HASH: &str = "0428ab8c8b0fe83d9e57fb6b26ff190173caad00ed7aeb683ce26cc4b56ea4bb";
    pub const PARQUEST_CHECKSUMMED_HASH: &str =
        "9c4db11437f11c3bbe25b39601069b8ed09b39f5f18ac29a13df4361240859d9";

    pub fn parquet() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_PARQUET)
    }

    pub fn jsonl() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_JSONL)
    }

    pub fn parquet_checksummed() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_PARQUET_CHECKSUMMED)
    }
}

pub mod manifest_empty {
    use super::local_uri;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::manifest::Header;
    use crate::manifest::Table;
    use crate::Res;

    const EMPTY_EMPTY: &str = "fixtures/header-empty-empty.jsonl";
    const EMPTY_NONE: &str = "fixtures/header-empty-none.jsonl";
    const EMPTY_NULL: &str = "fixtures/header-empty-null.jsonl";
    const NULL_EMPTY: &str = "fixtures/header-null-empty.jsonl";
    const NULL_NONE: &str = "fixtures/header-null-none.jsonl";
    const NULL_NULL: &str = "fixtures/header-null-null.jsonl";

    pub const EMPTY_EMPTY_TOP_HASH: &str =
        "0929824c58e90a6d2cc3ad2c7bdc66e34f43e8ed7063a6b48595a3834dd3ec99";
    pub const EMPTY_NONE_TOP_HASH: &str =
        "770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";
    pub const EMPTY_NULL_TOP_HASH: &str =
        "39ee9fb46019db2d8373c991d7881ba90bbb6a6c65417e108c295363794dec3c";
    pub const NULL_EMPTY_TOP_HASH: &str =
        "1a4cae60caad99aaf073c4292adfa510897c31c1d6adb44662925b9b143edbbf";
    pub const NULL_NONE_TOP_HASH: &str =
        "2a5a67156ca9238c14d12042db51c5b52260fdd5511b61ea89b58929d6e1769b";
    pub const NULL_NULL_TOP_HASH: &str =
        "fb53faf10edc3c9cc234f362c1a57d702213e869a20b887d9f6ed5439c936513";

    pub fn path_empty() -> Res<PathBuf> {
        local_uri(EMPTY_EMPTY)
    }

    pub fn path_empty_none() -> Res<PathBuf> {
        local_uri(EMPTY_NONE)
    }

    pub fn path_empty_null() -> Res<PathBuf> {
        local_uri(EMPTY_NULL)
    }

    pub fn path_null_empty() -> Res<PathBuf> {
        local_uri(NULL_EMPTY)
    }

    pub fn path_null_none() -> Res<PathBuf> {
        local_uri(NULL_NONE)
    }

    pub fn path_null() -> Res<PathBuf> {
        local_uri(NULL_NULL)
    }

    pub fn empty() -> Table {
        Table::new(
            Header::new(
                Some("".to_string()),
                Some(serde_json::Value::Object(serde_json::Map::new())),
                None,
            ),
            BTreeMap::new(),
        )
    }

    pub fn empty_none() -> Table {
        Table::new(
            Header::new(Some("".to_string()), None, None),
            BTreeMap::new(),
        )
    }

    pub fn empty_null() -> Table {
        Table::new(
            Header::new(Some("".to_string()), Some(serde_json::Value::Null), None),
            BTreeMap::new(),
        )
    }

    pub fn null_empty() -> Table {
        Table::new(
            Header::new(
                None,
                Some(serde_json::Value::Object(serde_json::Map::new())),
                None,
            ),
            BTreeMap::new(),
        )
    }

    pub fn null_none() -> Table {
        Table::new(Header::new(None, None, None), BTreeMap::new())
    }

    pub fn null() -> Table {
        Table::new(
            Header::new(None, Some(serde_json::Value::Null), None),
            BTreeMap::new(),
        )
    }
}

pub fn create_multihash(b64_str: &str) -> Res<Multihash<256>> {
    Ok(Multihash::wrap(
        MULTIHASH_SHA256_CHUNKED,
        &BASE64_STANDARD.decode(b64_str)?,
    )?)
}

pub mod manifest_with_objects_all_sizes {
    use std::path::PathBuf;

    use super::create_multihash;
    use super::local_uri;
    use super::objects;

    use crate::manifest::Row;
    use crate::manifest::Table;
    use crate::Res;

    const JSONL: &str = "fixtures/ref-manifest-sizes.jsonl";

    // Some physical keys are 'file://..."
    const PARQUET_LOCAL: &str = "fixtures/ref-manifest-sizes-local.parquet";

    // All physical keys are 's3://..."
    const PARQUET_REMOTE: &str = "fixtures/ref-manifest-sizes-remote.parquet";

    pub const TOP_HASH: &str = "a8287f20eb1e315a08ce08d9488dc1e8c75ba45d4549bb4351a74c92b217c3c0";

    pub fn jsonl_path() -> Res<PathBuf> {
        local_uri(JSONL)
    }

    pub fn parquet_local_path() -> Res<PathBuf> {
        local_uri(PARQUET_LOCAL)
    }

    pub fn parquet_remote_path() -> Res<PathBuf> {
        local_uri(PARQUET_REMOTE)
    }

    pub async fn manifest() -> Res<Table> {
        let mut manifest = Table::default();
        manifest
            .insert_record(Row {
                name: PathBuf::from("0mb.bin"),
                size: 0,
                hash: create_multihash(objects::ZERO_HASH_B64)?,
                ..Row::default()
            })
            .await?;
        manifest
            .insert_record(Row {
                name: PathBuf::from("bigger-than-8mb.txt"),
                size: 18874368,
                hash: create_multihash(objects::MORE_THAN_8MB_HASH_B64)?,
                ..Row::default()
            })
            .await?;
        manifest
            .insert_record(Row {
                name: PathBuf::from("equal-to-8mb.txt"),
                size: 8388608,
                hash: create_multihash(objects::EQUAL_TO_8MB_HASH_B64)?,
                ..Row::default()
            })
            .await?;
        manifest
            .insert_record(Row {
                name: PathBuf::from("less-then-8mb.txt"),
                size: 16,
                hash: create_multihash(objects::LESS_THAN_8MB_HASH_B64)?,
                ..Row::default()
            })
            .await?;
        manifest
            .insert_record(Row {
                name: PathBuf::from("one/two two/three three three/READ ME.md"),
                size: 20,
                hash: create_multihash(objects::NESTED_HASH_B64)?,
                ..Row::default()
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

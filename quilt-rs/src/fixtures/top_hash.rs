//! Top hash constants for manifest fixtures
//!
//! This module contains all the top hash constants used for manifest testing.
//! Each hash corresponds to a specific manifest configuration and can be used
//! to load the corresponding JSONL fixture file.

use super::local_uri;
use crate::Res;
use std::path::PathBuf;

/// Load manifest fixture by top hash
pub fn load_fixture(top_hash: &str) -> Res<PathBuf> {
    local_uri(&format!("fixtures/{}.jsonl", top_hash))
}

// EMPTY_* series: message=""

/// Manifest header: {"message":"","user_meta":{},"version":"v0"}
pub const EMPTY_EMPTY_TOP_HASH: &str =
    "0929824c58e90a6d2cc3ad2c7bdc66e34f43e8ed7063a6b48595a3834dd3ec99";

/// Manifest header: {"message":"","version":"v0"}
pub const EMPTY_NONE_TOP_HASH: &str =
    "770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";

/// Manifest header: {"message":"","user_meta":null,"version":"v0"}
pub const EMPTY_NULL_TOP_HASH: &str =
    "39ee9fb46019db2d8373c991d7881ba90bbb6a6c65417e108c295363794dec3c";

// NULL_* series: message=null

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

// Additional workflow combinations with different user_meta values

/// Manifest header: {"message":"","version":"v0","workflow":{"config":"s3://workflow/config","id":null}}
pub const EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH: &str =
    "88df0e39d2ecb9493f44ef30af4e7c6f6e9d5daa0b23e468aa1643a407bbc81c";

/// Manifest header: {"message":"","user_meta":null,"version":"v0","workflow":{"config":"s3://workflow/config","id":null}}
pub const EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH: &str =
    "129804b4ad21520c21d21c5f2916f549b4e2a7cb106e4efe88c9676270ac00f1";

/// Manifest header: {"message":"Initial","user_meta":{"key":"value"},"version":"v0","workflow":{"config":"s3://workflow/config","id":null}}
pub const INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH: &str =
    "7b224912378c80eef0f6255d911b4e7a51c287ea89cc9fee32cee26e56090b0c";

/// Manifest header: {"message":"Initial","version":"v0","workflow":{"config":"s3://workflow/config","id":"test-workflow","schemas":{"test-schema":"s3://bucket/workflows/test.json"}}}
pub const INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH: &str =
    "f9b5b98503dc3feb22b5fc94bd8ff474efb39adcd9fa2a9f6b3199de536ec5ca";

// ROWS series: with header + rows combinations

/// Single row manifest with default header
/// Hash: objects::LESS_THAN_8MB_HASH_B64 (16 bytes)
/// JSON:
/// {"message":"","user_meta":{},"version":"v0"}
/// {"logical_key":"data.txt","physical_keys":["s3://bucket/data.txt"],"hash":{"type":"sha2-256-chunked","value":"Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="},"size":16,"meta":{"type":"text"}}
pub const SINGLE_ROW_TOP_HASH: &str =
    "9adfc26d4d85bc1a31bd9b45af1c78647415d580a4196becd85f9e4e793c5824";

/// Multiple rows manifest with default header
/// Hashes: ZERO_HASH_B64 (0 bytes), EQUAL_TO_8MB_HASH_B64 (8388608 bytes), MORE_THAN_8MB_HASH_B64 (18874368 bytes)
/// JSON:
/// {"message":"","user_meta":{},"version":"v0"}
/// {"logical_key":"config.json","physical_keys":["s3://bucket/config.json"],"hash":{"type":"sha2-256-chunked","value":"47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="},"size":0,"meta":{"format":"json"}}
/// {"logical_key":"data/file.csv","physical_keys":["s3://bucket/data/file.csv"],"hash":{"type":"sha2-256-chunked","value":"7V3rZ3Q/AmAYax2wsQBZbc7N1EMIxlxRyMiMthGRdwg="},"size":8388608,"meta":null}
/// {"logical_key":"images/photo.jpg","physical_keys":["s3://bucket/images/photo.jpg"],"hash":{"type":"sha2-256-chunked","value":"T+rt/HKRJOiAkEGXKvc+DhCwRcrZiDrFkjKonDT1zgs="},"size":18874368,"meta":{"height":1080,"width":1920}}
pub const MULTIPLE_ROWS_TOP_HASH: &str =
    "b86d9eb02bd108cdd1823d53558c752b9466928c37af7655e4080595633ead7e";

/// Mixed hash types manifest with default header: SHA256, sha2-256-chunked, CRC64NVME
/// Hash values: "7465737464617461000000000000000000000000000000000000000000000000" (8 bytes), LESS_THAN_8MB_HASH_B64 (16 bytes), "dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA" (32 bytes)
/// JSON:
/// {"message":"","user_meta":{},"version":"v0"}
/// {"logical_key":"file1.txt","physical_keys":["s3://bucket/file1.txt"],"hash":{"type":"SHA256","value":"7465737464617461000000000000000000000000000000000000000000000000"},"size":8,"meta":{}}
/// {"logical_key":"file2.txt","physical_keys":["s3://bucket/file2.txt"],"hash":{"type":"sha2-256-chunked","value":"Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="},"size":16,"meta":{}}
/// {"logical_key":"file3.txt","physical_keys":["s3://bucket/file3.txt"],"hash":{"type":"CRC64NVME","value":"dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA"},"size":32,"meta":{}}
pub const MIXED_HASH_TYPES_TOP_HASH: &str =
    "45e16dfe0c880236eb9145d20ff246fa190788be2c8b517bf20abd8be6165c10";

/// Hash normalization equivalence test - tests that different JSON representations produce same hash
/// Tests: meta:{} vs meta:null vs meta:None, and key order normalization {"alpha":"first","beta":"second"} vs {"beta":"second","alpha":"first"}
/// JSON (normalized form):
/// {"message":"","user_meta":{},"version":"v0"}
/// {"logical_key":"test1.txt","physical_keys":["s3://bucket/test1.txt"],"hash":{"type":"sha2-256-chunked","value":"47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="},"size":0,"meta":{}}
/// {"logical_key":"test2.txt","physical_keys":["s3://bucket/test2.txt"],"hash":{"type":"sha2-256-chunked","value":"Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="},"size":16,"meta":{"alpha":"first","beta":"second"}}
pub const NORMALIZED_EQUIVALENCE_TOP_HASH: &str =
    "10c3b62176b4fbb25b4988181bb65e3861087403f36f13c8adb66bce52d6471b";

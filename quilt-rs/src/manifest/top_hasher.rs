//! Top-level manifest hash calculation utilities.
//!
//! This module provides the `TopHasher` struct and related functions for calculating
//! deterministic hashes of manifest headers and rows to create top-level manifest hashes.

use std::fmt;

use aws_smithy_checksums::ChecksumAlgorithm;
use serde::Serialize;

use crate::Res;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
#[cfg(test)]
use crate::manifest::Workflow;
#[cfg(test)]
use crate::manifest::WorkflowId;

/// The text of one part of the top hash, in quilt3's top-hash encoding:
/// keys sorted at every level, no spaces, non-ASCII escaped, and floats in
/// the form [`hash_float_text`] gives. quilt3's `Package._calculate_top_hash`
/// is the reference.
fn encode_hash_part<T: Serialize>(value: &T) -> Res<String> {
    let mut out = String::new();
    write_hash_json(&mut out, &serde_json::to_value(value)?);
    Ok(out)
}

fn write_hash_json(out: &mut String, value: &serde_json::Value) {
    use serde_json::Value;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => out.push_str(&hash_float_text(f)),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => write_hash_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_hash_json(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Keys sort by code point, which is the byte order of UTF-8.
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_hash_string(out, key);
                out.push(':');
                write_hash_json(out, item);
            }
            out.push('}');
        }
    }
}

/// A JSON string with everything outside `' '..='~'` escaped: the short forms
/// for quote, backslash and five controls, else `\uXXXX` in lowercase hex,
/// as a UTF-16 surrogate pair above U+FFFF.
fn write_hash_string(out: &mut String, s: &str) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// A float as the top hash writes it: the shortest digits that read back as
/// the same float, the nearer one on a tie. Plain when the exponent is in
/// `-4..16` (`0.0001`, `1.0`), else with a signed exponent of at least two
/// digits (`1e-07`, `1.5e+300`).
fn hash_float_text(f: f64) -> String {
    // ryu breaks ties toward the nearer value; Rust's `{:e}` does not always.
    // Its text mixes plain and exponent forms, so read digits and exponent.
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(f);
    let (sign, text) = match text.strip_prefix('-') {
        Some(t) => ("-", t),
        None => ("", text),
    };
    let (mantissa, exp) = text.split_once('e').unwrap_or((text, "0"));
    let exp: i32 = exp.parse().expect("ryu writes an integer exponent");
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = format!("{int_part}{frac_part}");
    let digits = all_digits.trim_start_matches('0');
    let leading_zeros = all_digits.len() - digits.len();
    let digits = digits.trim_end_matches('0');
    if digits.is_empty() {
        return format!("{sign}0.0");
    }
    let digits = digits.to_string();
    // Exponent of the first significant digit.
    let exp = exp + i32::try_from(int_part.len()).unwrap_or_default()
        - 1
        - i32::try_from(leading_zeros).unwrap_or_default();

    if (-4..16).contains(&exp) {
        let point = exp + 1; // digits before the decimal point
        if point <= 0 {
            let zeros = "0".repeat(usize::try_from(-point).unwrap_or_default());
            format!("{sign}0.{zeros}{digits}")
        } else {
            let point = usize::try_from(point).unwrap_or_default();
            if digits.len() > point {
                format!("{sign}{}.{}", &digits[..point], &digits[point..])
            } else {
                let zeros = "0".repeat(point - digits.len());
                format!("{sign}{digits}{zeros}.0")
            }
        }
    } else {
        let exp_sign = if exp < 0 { '-' } else { '+' };
        let mantissa = if digits.len() > 1 {
            format!("{}.{}", &digits[..1], &digits[1..])
        } else {
            digits
        };
        format!("{sign}{mantissa}e{exp_sign}{:02}", exp.abs())
    }
}

fn serialize_manifest_header(
    manifest_header: &ManifestHeader,
) -> Res<serde_json::Map<String, serde_json::Value>> {
    let mut header_meta = serde_json::Map::new();

    // Handle message
    if let Some(message) = &manifest_header.message {
        header_meta.insert("message".to_string(), serde_json::to_value(message)?);
    } else {
        header_meta.insert("message".to_string(), serde_json::Value::Null);
    }

    // Handle user_meta - preserve null vs missing distinction for header
    if let Some(user_meta) = &manifest_header.user_meta {
        let u = match user_meta {
            serde_json::Value::Object(m) => {
                let mut m = m.clone();
                m.values_mut().for_each(serde_json::Value::sort_all_objects);
                m.sort_keys();
                serde_json::Value::Object(m)
            }
            _ => user_meta.clone(),
        };
        header_meta.insert("user_meta".into(), u);
    }

    header_meta.insert(
        "version".to_string(),
        serde_json::Value::String(manifest_header.version.clone()),
    );

    if let Some(workflow) = &manifest_header.workflow {
        header_meta.insert("workflow".to_string(), serde_json::to_value(workflow)?);
    }

    Ok(header_meta)
}

/// An entry's `meta` as the top hash reads it.
///
/// An absent `meta` and every empty or false one (`null`, `false`, `0`, `0.0`,
/// `""`, `[]`, `{}`) are `{}`; anything else is kept as is.
pub(crate) fn normalized_entry_meta(meta: Option<&serde_json::Value>) -> serde_json::Value {
    use serde_json::Value;
    let is_falsy = match meta {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    };
    if is_falsy {
        return Value::Object(serde_json::Map::default());
    }
    let mut value = meta.cloned().unwrap_or_default();
    value.sort_all_objects();
    value
}

pub fn serialize_manifest_row_entry(
    manifest_row: &ManifestRow,
) -> Res<serde_json::Map<String, serde_json::Value>> {
    let meta = normalized_entry_meta(manifest_row.meta.as_ref());

    Ok(serde_json::Map::from_iter([
        (
            "hash".to_string(),
            serde_json::to_value(&manifest_row.hash)?,
        ),
        (
            "logical_key".to_string(),
            serde_json::to_value(&manifest_row.logical_key)?,
        ),
        ("meta".to_string(), meta),
        (
            "size".to_string(),
            serde_json::Value::Number(manifest_row.size.into()),
        ),
    ]))
}

/// Helper for creating `top_hash`
pub struct TopHasher {
    pub hasher: Box<dyn aws_smithy_checksums::Checksum>,
}

impl fmt::Debug for TopHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TopHasher")
            .field("hasher", &"<aws_smithy_checksums::Checksum>")
            .finish()
    }
}

impl Default for TopHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl TopHasher {
    #[must_use]
    pub fn new() -> Self {
        TopHasher {
            hasher: ChecksumAlgorithm::Sha256.into_impl(),
        }
    }

    /// Append `ManifestHeader` to the hasher
    pub fn append_header(&mut self, manifest_header: &ManifestHeader) -> Res {
        let value = serialize_manifest_header(manifest_header)?;
        let value_str = encode_hash_part(&value)?;
        self.hasher.update(value_str.as_bytes());
        Ok(())
    }

    /// Append `ManifestRow` to the hasher
    pub fn append(&mut self, manifest_row: &ManifestRow) -> Res {
        let value = serialize_manifest_row_entry(manifest_row)?;
        let value_str = encode_hash_part(&value)?;
        self.hasher.update(value_str.as_bytes());
        Ok(())
    }

    /// Consume `self` and return `top_hash`
    #[must_use]
    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use test_log::test;

    use crate::Res;
    use crate::fixtures;
    use crate::fixtures::objects;
    use crate::fixtures::top_hash;
    use crate::object_hash::Crc64Hash;
    use crate::object_hash::Sha256ChunkedHash;
    use crate::object_hash::Sha256Hash;

    /// Expected text is what quilt3's top-hash encoder writes for the float.
    #[test]
    fn floats_are_written_as_quilt3_writes_them() {
        for (input, expected) in [
            ("1e-7", "1e-07"),
            ("-2.5e-8", "-2.5e-08"),
            ("1e-5", "1e-05"),
            ("0.0001", "0.0001"),
            ("0.00012345", "0.00012345"),
            ("0.1", "0.1"),
            ("1.0", "1.0"),
            ("-0.0", "-0.0"),
            ("0.0", "0.0"),
            ("100.0", "100.0"),
            ("123456789.123", "123456789.123"),
            ("1e15", "1000000000000000.0"),
            ("999999999999999.9", "999999999999999.9"),
            ("1e16", "1e+16"),
            ("1.5e300", "1.5e+300"),
            ("5e-324", "5e-324"),
            ("1.7976931348623157e308", "1.7976931348623157e+308"),
            ("3e22", "3e+22"),
            ("0.30000000000000004", "0.30000000000000004"),
            ("2.5", "2.5"),
            ("1234567890123456.0", "1234567890123456.0"),
            ("12345678901234567.0", "1.2345678901234568e+16"),
            ("-1e-4", "-0.0001"),
            ("9.999999999999999e-5", "9.999999999999999e-05"),
            // Two shortest candidates; the nearer one wins.
            ("667962307169479.2", "667962307169479.2"),
        ] {
            let f: f64 = input.parse().unwrap();
            assert_eq!(hash_float_text(f), expected, "{input}");
        }
    }

    #[test]
    fn test_manifest_header_default_no_rows() -> Res {
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_none_no_rows() -> Res {
        let header = ManifestHeader {
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_null_no_rows() -> Res {
        let header = ManifestHeader {
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_empty_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_none_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_null_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_none_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_null_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_complex_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"author": "user", "timestamp": "2024-01-01"})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_large_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({
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
            })),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    schemas: BTreeMap::from([(
                        "test-schema".to_string(),
                        "s3://bucket/workflows/test.json".parse()?,
                    )]),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_entries_workflow_no_rows() -> Res {
        // An entries-schema-only stamp must hash identically to quilt3's — the
        // constant is corroborated against quilt3, not copied from this output.
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "entries-wf".to_string(),
                    schemas: BTreeMap::from([(
                        "entries-schema".to_string(),
                        "s3://bucket/workflows/entries.json".parse()?,
                    )]),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        assert_eq!(
            top_hasher.finalize(),
            top_hash::EMPTY_EMPTY_ENTRIES_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_dual_workflow_no_rows() -> Res {
        // Both schemas stamped; hash corroborated against quilt3.
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "dual-wf".to_string(),
                    schemas: BTreeMap::from([
                        (
                            "meta-schema".to_string(),
                            "s3://bucket/workflows/meta.json".parse()?,
                        ),
                        (
                            "entries-schema".to_string(),
                            "s3://bucket/workflows/entries.json".parse()?,
                        ),
                    ]),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        assert_eq!(
            top_hasher.finalize(),
            top_hash::EMPTY_EMPTY_DUAL_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    schemas: BTreeMap::from([(
                        "test-schema".to_string(),
                        "s3://bucket/workflows/test.json".parse()?,
                    )]),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_none_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_null_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some(String::new()),
            user_meta: Some(serde_json::Value::Null),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_meta_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_none_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    schemas: BTreeMap::from([(
                        "test-schema".to_string(),
                        "s3://bucket/workflows/test.json".parse()?,
                    )]),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_checksummed_manifest_top_hash_direct() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        for i in 0..10 {
            let manifest_row = ManifestRow {
                logical_key: crate::LogicalKey::new(format!("e0-{i}.txt"))?,
                physical_key: "ignored".to_string(),
                hash: Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?
                    .into(),
                size: 29,
                meta: Some(serde_json::Value::Null),
            };
            top_hasher.append(&manifest_row)?;
        }

        let calculated_hash = top_hasher.finalize();
        assert_eq!(calculated_hash, fixtures::manifest::TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_single_row() -> Res {
        // Single row with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let manifest_row = ManifestRow {
            logical_key: crate::LogicalKey::new("data.txt")?,
            physical_key: "s3://bucket/data.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"type": "text"})),
        };
        top_hasher.append(&manifest_row)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::SINGLE_ROW_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_mixed_hash_types() -> Res {
        // Mixed hash types with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        // Row 1: SHA256 hash (legacy format)
        let row1 = ManifestRow {
            logical_key: crate::LogicalKey::new("file1.txt")?,
            physical_key: "s3://bucket/file1.txt".to_string(),
            hash: Sha256Hash::try_from(
                "7465737464617461000000000000000000000000000000000000000000000000",
            )?
            .into(),
            size: 8,
            meta: None,
        };
        top_hasher.append(&row1)?;

        // Row 2: SHA256-chunked hash (current format)
        let row2 = ManifestRow {
            logical_key: crate::LogicalKey::new("file2.txt")?,
            physical_key: "s3://bucket/file2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: None,
        };
        top_hasher.append(&row2)?;

        // Row 3: CRC64-NVMe hash (newest format)
        let row3 = ManifestRow {
            logical_key: crate::LogicalKey::new("file3.txt")?,
            physical_key: "s3://bucket/file3.txt".to_string(),
            hash: Crc64Hash::try_from("dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA")?.into(),
            size: 32,
            meta: None,
        };
        top_hasher.append(&row3)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::MIXED_HASH_TYPES_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_hash_normalization_equivalence() -> Res {
        // Test that different JSON representations produce the same hash
        // when they represent equivalent data structures

        // First manifest: meta: {}, logical_key first
        let header1 = ManifestHeader::default();
        let mut top_hasher1 = TopHasher::new();
        top_hasher1.append_header(&header1)?;

        let empty_obj_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test1.txt")?,
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::json!({})), // Empty object
        };

        let sorted_keys_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test2.txt")?,
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"alpha": "first", "beta": "second"})), // Keys in alphabetical order
        };

        top_hasher1.append(&empty_obj_meta_row)?;
        top_hasher1.append(&sorted_keys_meta_row)?;
        let hash1 = top_hasher1.finalize();

        // Second manifest: meta: null (becomes {}), different key order, field order doesn't matter for hashing
        let header2 = ManifestHeader::default();
        let mut top_hasher2 = TopHasher::new();
        top_hasher2.append_header(&header2)?;

        let null_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test1.txt")?,
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::Value::Null), // Null becomes {}
        };

        let unsorted_keys_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test2.txt")?,
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"beta": "second", "alpha": "first"})), // Keys in different order
        };

        top_hasher2.append(&null_meta_row)?;
        top_hasher2.append(&unsorted_keys_meta_row)?;
        let hash2 = top_hasher2.finalize();

        // Third manifest: meta: None (becomes {})
        let header3 = ManifestHeader::default();
        let mut top_hasher3 = TopHasher::new();
        top_hasher3.append_header(&header3)?;

        let missing_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test1.txt")?,
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: None, // None becomes {}
        };

        let reordered_keys_meta_row = ManifestRow {
            logical_key: crate::LogicalKey::new("test2.txt")?,
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"beta": "second", "alpha": "first"})), // Same as above after normalization
        };

        top_hasher3.append(&missing_meta_row)?;
        top_hasher3.append(&reordered_keys_meta_row)?;
        let hash3 = top_hasher3.finalize();

        // All three should produce the same hash despite different representations
        assert_eq!(
            hash1, hash2,
            "Empty object {{}} and null should normalize to same hash"
        );
        assert_eq!(
            hash1, hash3,
            "Empty object {{}}, null, and None should normalize to same hash"
        );
        assert_eq!(
            hash2, hash3,
            "All meta empty representations should normalize to same hash"
        );

        // Test that the normalized hash matches our expected constant
        assert_eq!(hash1, top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_multiple_rows() -> Res {
        // Multiple rows with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        // Row 1: Small file
        let row1 = ManifestRow {
            logical_key: crate::LogicalKey::new("config.json")?,
            physical_key: "s3://bucket/config.json".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::json!({"format": "json"})),
        };
        top_hasher.append(&row1)?;

        // Row 2: Medium file
        let row2 = ManifestRow {
            logical_key: crate::LogicalKey::new("data/file.csv")?,
            physical_key: "s3://bucket/data/file.csv".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::EQUAL_TO_8MB_HASH_B64)?.into(),
            size: 8_388_608,
            meta: Some(serde_json::Value::Null),
        };
        top_hasher.append(&row2)?;

        // Row 3: Large file
        let row3 = ManifestRow {
            logical_key: crate::LogicalKey::new("images/photo.jpg")?,
            physical_key: "s3://bucket/images/photo.jpg".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::MORE_THAN_8MB_HASH_B64)?.into(),
            size: 18_874_368,
            meta: Some(serde_json::json!({"width": 1920, "height": 1080})),
        };
        top_hasher.append(&row3)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);

        Ok(())
    }
}

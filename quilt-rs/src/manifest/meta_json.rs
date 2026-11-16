//! Numbers in metadata that `serde_json::Value` cannot hold as quilt3 does,
//! and the one writer of metadata text, for the top hash and for manifests.
//!
//! quilt3 reads a number written without a fraction or exponent as a Python
//! `int`, of any size, and writes it back digit for digit; `-0` is the int
//! `0`. `serde_json::Value` (without its `arbitrary_precision` feature, which
//! cannot be turned on for one crate) holds such a number outside the 64-bit
//! range as a float, and `-0` as the float `-0.0`. So reading a manifest keeps
//! the first as [`RawNumbers`] next to the value and turns the second into
//! `0`, and writing puts the kept text back.

use std::collections::BTreeMap;

use serde_json::Number;
use serde_json::Value;

use super::top_hasher::hash_float_text;
use super::top_hasher::write_hash_string;

/// The integers in a JSON value that are outside the 64-bit range, as the
/// text they were read from, by their place in the value.
///
/// Only numbers that `serde_json::Value` would hold wrongly are kept, so this
/// is empty for nearly every manifest. A kept number applies only while the
/// value at its place is still a float equal to the one it was read as, so a
/// value edited after reading never gets stale text. The guard is f64
/// equality: replacing the number with another that is the same float (the
/// neighbouring integer, or `1.8446744073709552e19` for `18446744073709551616`)
/// keeps the old text.
///
/// `ManifestRow`, `ManifestHeader` and `DirMeta` compare their `RawNumbers` in
/// `PartialEq`, so two values that are the same float but whose kept text
/// differs are different. They are not serialized: a `Manifest` written
/// through serde, such as the CLI's JSON output, has those numbers as floats.
/// [`Manifest::to_jsonlines`](super::Manifest::to_jsonlines) is the writer
/// that keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawNumbers(BTreeMap<String, String>);

impl RawNumbers {
    /// Whether no number is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `pointer` is `prefix` or a value under it.
    fn at_or_under(pointer: &str, prefix: &str) -> bool {
        pointer == prefix || pointer.starts_with(&format!("{prefix}/"))
    }

    /// The numbers at or under `prefix`.
    #[must_use]
    pub fn only(&self, prefix: &str) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(pointer, _)| Self::at_or_under(pointer, prefix))
                .map(|(pointer, text)| (pointer.clone(), text.clone()))
                .collect(),
        )
    }

    /// The numbers not at or under `prefix`.
    #[must_use]
    pub fn without(&self, prefix: &str) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(pointer, _)| !Self::at_or_under(pointer, prefix))
                .map(|(pointer, text)| (pointer.clone(), text.clone()))
                .collect(),
        )
    }

    /// The numbers `value` still holds as the same float, so a replaced
    /// subtree's text is dropped and a field that was kept is not.
    #[must_use]
    pub fn still_in(&self, value: &Value) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(pointer, _)| {
                    value
                        .pointer(pointer)
                        .and_then(Value::as_number)
                        .is_some_and(|number| self.text_for(pointer, number).is_some())
                })
                .map(|(pointer, text)| (pointer.clone(), text.clone()))
                .collect(),
        )
    }

    /// All of `self`'s numbers and `other`'s; `other` wins at the same place.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut numbers = self.0.clone();
        numbers.extend(other.0.iter().map(|(k, v)| (k.clone(), v.clone())));
        Self(numbers)
    }

    /// Parse JSON text as quilt3 reads it: the value, and the integers it
    /// cannot hold.
    ///
    /// # Errors
    ///
    /// The text is not JSON.
    pub fn parse(text: &str) -> Result<(Value, Self), serde_json::Error> {
        let mut value: Value = serde_json::from_str(text)?;
        let numbers = if holds_float(&value) {
            Self::take(&mut value, Scanner::new(text).scan_all())
        } else {
            Self::default()
        };
        Ok((value, numbers))
    }

    /// The numbers kept for the value under `key` in the JSON object `line`,
    /// once `value` has been read from it; `-0` in `value` becomes `0`.
    ///
    /// Both a whole number beyond 64 bits and `-0` are floats in `value`, so
    /// a value with no float needs no second look at the text; otherwise only
    /// the text of `key`'s value is scanned.
    pub(crate) fn read(value: &mut Value, line: &str, key: &str) -> Self {
        if holds_float(value) {
            Self::take(value, Scanner::new(line).scan_key(key))
        } else {
            Self::default()
        }
    }

    /// Keep the scanned integers that are what `value` holds, and turn the
    /// `-0` it holds into `0`.
    fn take(value: &mut Value, found: BTreeMap<String, String>) -> Self {
        let mut kept = BTreeMap::new();
        for (pointer, number) in found {
            if number == "-0" {
                if let Some(slot) = value.pointer_mut(&pointer)
                    && slot
                        .as_f64()
                        .is_some_and(|f| f == 0.0 && f.is_sign_negative())
                {
                    *slot = Value::from(0);
                }
            } else if let Some(Value::Number(n)) = value.pointer(&pointer)
                && n.is_f64()
                && number.parse::<f64>().ok() == n.as_f64()
            {
                kept.insert(pointer, number);
            }
        }
        Self(kept)
    }

    /// The same numbers for the value that sits under `prefix` (a JSON
    /// pointer) in a larger one.
    pub(crate) fn under(&self, prefix: &str) -> Self {
        Self(
            self.0
                .iter()
                .map(|(pointer, text)| (format!("{prefix}{pointer}"), text.clone()))
                .collect(),
        )
    }

    /// The text to write for `number` at `pointer`, when one is kept and
    /// still describes it.
    fn text_for(&self, pointer: &str, number: &Number) -> Option<&str> {
        let text = self.0.get(pointer)?;
        (number.is_f64() && text.parse::<f64>().ok() == number.as_f64()).then_some(text.as_str())
    }
}

/// How [`write_json`] writes.
#[derive(Clone, Copy)]
pub(crate) enum Style {
    /// quilt3's top-hash encoding: keys sorted at every level, non-ASCII
    /// escaped.
    Hash,
    /// quilt3's manifest encoding: keys as they are, non-ASCII as it is.
    Manifest,
}

/// `value` as text, with floats as Python writes them and the kept integers
/// as they were read.
pub(crate) fn write_json(out: &mut String, value: &Value, exact: &RawNumbers, style: Style) {
    let mut pointer = String::new();
    write(out, value, exact, style, &mut pointer);
}

fn write(out: &mut String, value: &Value, exact: &RawNumbers, style: Style, pointer: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(text) = exact.text_for(pointer, n) {
                out.push_str(text);
            } else if let (true, Some(f)) = (n.is_f64(), n.as_f64()) {
                out.push_str(&hash_float_text(f));
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => write_string(out, s, style),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let at = pointer.len();
                if !exact.is_empty() {
                    pointer.push('/');
                    pointer.push_str(&i.to_string());
                }
                write(out, item, exact, style, pointer);
                pointer.truncate(at);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            if matches!(style, Style::Hash) {
                // Keys sort by code point, which is the byte order of UTF-8.
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key, style);
                out.push(':');
                let at = pointer.len();
                if !exact.is_empty() {
                    pointer.push('/');
                    pointer.push_str(&escape_pointer(key));
                }
                write(out, item, exact, style, pointer);
                pointer.truncate(at);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str, style: Style) {
    match style {
        Style::Hash => write_hash_string(out, s),
        Style::Manifest => {
            out.push_str(&serde_json::to_string(s).expect("a string is always JSON"));
        }
    }
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Whether `value` holds a float: where `serde_json` puts a whole number
/// beyond 64 bits and `-0`.
fn holds_float(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_f64(),
        Value::Array(items) => items.iter().any(holds_float),
        Value::Object(map) => map.values().any(holds_float),
        _ => false,
    }
}

/// A scan of JSON text for the integers `serde_json::Value` holds wrongly:
/// `-0` and those outside the 64-bit range, by JSON pointer.
///
/// The text at a place is what its last occurrence is, as in `serde_json` and
/// Python: a later key of the same name drops what an earlier one kept.
struct Scanner<'a> {
    text: &'a str,
    pos: usize,
    pointer: String,
    found: BTreeMap<String, String>,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Scanner {
            text,
            pos: 0,
            pointer: String::new(),
            found: BTreeMap::new(),
        }
    }

    /// Every such integer in the text, which is JSON that `serde_json` has
    /// read; a malformed text just ends the scan.
    fn scan_all(mut self) -> BTreeMap<String, String> {
        let _ = self.value();
        self.found
    }

    /// Those in the value of `key` in the JSON object that is the text, by
    /// pointer within that value. The other values are skipped, not read.
    fn scan_key(mut self, key: &str) -> BTreeMap<String, String> {
        if let Some(start) = self.seek_key(key) {
            self.pos = start;
            let _ = self.value();
        }
        self.found
    }

    /// Where the last value of `key` starts in the object that is the text.
    fn seek_key(&mut self, key: &str) -> Option<usize> {
        self.skip_space();
        if self.peek()? != b'{' {
            return None;
        }
        self.pos += 1;
        let mut start = None;
        loop {
            self.skip_space();
            if self.peek()? == b'}' {
                return start;
            }
            let raw = self.string()?;
            let is_key = if raw.contains('\\') {
                serde_json::from_str::<String>(raw).ok()? == key
            } else {
                raw.get(1..raw.len() - 1) == Some(key)
            };
            self.skip_space();
            if self.peek()? != b':' {
                return None;
            }
            self.pos += 1;
            self.skip_space();
            if is_key {
                start = Some(self.pos);
            }
            self.skip_value()?;
            self.skip_space();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => return start,
                _ => return None,
            }
        }
    }

    /// Past the value at the position, whatever it is.
    fn skip_value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string().map(drop),
            b'{' | b'[' => {
                let mut depth = 0_usize;
                loop {
                    match self.peek()? {
                        b'"' => {
                            self.string()?;
                        }
                        b'{' | b'[' => {
                            depth += 1;
                            self.pos += 1;
                        }
                        b'}' | b']' => {
                            depth -= 1;
                            self.pos += 1;
                            if depth == 0 {
                                return Some(());
                            }
                        }
                        _ => self.pos += 1,
                    }
                }
            }
            _ => {
                while !matches!(self.peek()?, b',' | b'}' | b']') {
                    self.pos += 1;
                }
                Some(())
            }
        }
    }

    /// A value starts at the current place: whatever was kept for the place
    /// is not its value any more.
    fn start_value(&mut self) {
        if !self.found.is_empty() {
            self.found.remove(&self.pointer);
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// The quoted string at the position, raw, and past it.
    fn string(&mut self) -> Option<&'a str> {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.peek()? {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        self.text.get(start..self.pos)
    }

    fn value(&mut self) -> Option<()> {
        self.skip_space();
        self.start_value();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(drop),
            b't' | b'f' | b'n' => {
                while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                    self.pos += 1;
                }
                Some(())
            }
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Option<()> {
        self.pos += 1;
        self.skip_space();
        if self.peek()? == b'}' {
            self.pos += 1;
            return Some(());
        }
        loop {
            self.skip_space();
            let key: String = serde_json::from_str(self.string()?).ok()?;
            self.skip_space();
            if self.peek()? != b':' {
                return None;
            }
            self.pos += 1;
            let at = self.pointer.len();
            self.pointer.push('/');
            self.pointer.push_str(&escape_pointer(&key));
            self.value()?;
            self.pointer.truncate(at);
            self.skip_space();
            let next = self.peek()?;
            self.pos += 1;
            match next {
                b',' => {}
                b'}' => return Some(()),
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<()> {
        self.pos += 1;
        self.skip_space();
        if self.peek()? == b']' {
            self.pos += 1;
            return Some(());
        }
        let mut index = 0;
        loop {
            let at = self.pointer.len();
            self.pointer.push('/');
            self.pointer.push_str(&index.to_string());
            self.value()?;
            self.pointer.truncate(at);
            index += 1;
            self.skip_space();
            let next = self.peek()?;
            self.pos += 1;
            match next {
                b',' => {}
                b']' => return Some(()),
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<()> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'))
        {
            self.pos += 1;
        }
        let token = self.text.get(start..self.pos).filter(|t| !t.is_empty())?;
        let is_integer = token.bytes().all(|b| b == b'-' || b.is_ascii_digit());
        let fits = token.parse::<i64>().is_ok() || token.parse::<u64>().is_ok();
        if is_integer && (token == "-0" || !fits) {
            self.found.insert(self.pointer.clone(), token.to_string());
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    fn written(value: &Value, exact: &RawNumbers, style: Style) -> String {
        let mut out = String::new();
        write_json(&mut out, value, exact, style);
        out
    }

    #[test]
    fn integers_beyond_64_bits_are_kept_as_written() {
        let text = r#"{"a": [1, 18446744073709551616, {"b": -9223372036854775809}], "c": 18446744073709551615, "d": -9223372036854775808}"#;
        let (value, numbers) = RawNumbers::parse(text).unwrap();
        assert_eq!(
            numbers.0,
            BTreeMap::from([
                ("/a/1".to_string(), "18446744073709551616".to_string()),
                ("/a/2/b".to_string(), "-9223372036854775809".to_string()),
            ]),
            "the ones in range stay exact in the value"
        );
        let expected = r#"{"a":[1,18446744073709551616,{"b":-9223372036854775809}],"c":18446744073709551615,"d":-9223372036854775808}"#;
        assert_eq!(written(&value, &numbers, Style::Hash), expected);
        assert_eq!(written(&value, &numbers, Style::Manifest), expected);
    }

    #[test]
    fn negative_zero_is_zero_and_a_float_is_still_a_float() {
        let (value, numbers) = RawNumbers::parse(r#"{"i": -0, "f": -0.0, "l": [-0]}"#).unwrap();
        assert!(numbers.is_empty());
        assert_eq!(value["i"], serde_json::json!(0));
        assert_eq!(value["l"][0], serde_json::json!(0));
        assert_eq!(
            written(&value, &numbers, Style::Hash),
            r#"{"f":-0.0,"i":0,"l":[0]}"#
        );
    }

    #[test]
    fn nothing_in_a_string_is_a_number() {
        let text = r#"{"s": "-0 and 18446744073709551616 and \" 99999999999999999999", "n": 1}"#;
        let (value, numbers) = RawNumbers::parse(text).unwrap();
        assert!(numbers.is_empty());
        assert_eq!(value["n"], serde_json::json!(1));
    }

    #[test]
    fn keys_with_slashes_and_tildes_have_their_own_places() {
        let text = r#"{"a/b": 18446744073709551616, "~": -18446744073709551616, "~1": 7}"#;
        let (value, numbers) = RawNumbers::parse(text).unwrap();
        assert_eq!(
            written(&value, &numbers, Style::Manifest),
            r#"{"a/b":18446744073709551616,"~":-18446744073709551616,"~1":7}"#
        );
    }

    #[test]
    fn the_last_of_two_equal_keys_wins_as_in_quilt3() {
        for (text, expected) in [
            (
                r#"{"a": 1, "a": 18446744073709551616}"#,
                r#"{"a":18446744073709551616}"#,
            ),
            (r#"{"a": 18446744073709551616, "a": 1}"#, r#"{"a":1}"#),
            (r#"{"a": -0, "a": 5}"#, r#"{"a":5}"#),
        ] {
            let (value, numbers) = RawNumbers::parse(text).unwrap();
            assert_eq!(written(&value, &numbers, Style::Hash), expected, "{text}");
        }
    }

    /// Each pair is what Python's `json.loads` and `json.dumps(sort_keys=True,
    /// separators=(",", ":"))` give for the text.
    #[test]
    fn the_last_occurrence_decides_what_is_kept_whatever_it_is() {
        for (text, expected) in [
            (
                r#"{"a":18446744073709551617,"a":1.8446744073709552e19}"#,
                r#"{"a":1.8446744073709552e+19}"#,
            ),
            (
                r#"{"a":1.8446744073709552e19,"a":18446744073709551617}"#,
                r#"{"a":18446744073709551617}"#,
            ),
            (
                r#"{"a":18446744073709551617,"a":{"b":1}}"#,
                r#"{"a":{"b":1}}"#,
            ),
            (
                r#"{"a":{"b":18446744073709551617},"a":{"c":1}}"#,
                r#"{"a":{"c":1}}"#,
            ),
            (r#"{"a":-0,"a":-0.0}"#, r#"{"a":-0.0}"#),
            (r#"{"a":-0.0,"a":-0}"#, r#"{"a":0}"#),
            (
                r#"{"o":{"n":18446744073709551617,"f":1.5},"o":{"n":2.5e19,"f":1.5}}"#,
                r#"{"o":{"f":1.5,"n":2.5e+19}}"#,
            ),
            (
                r#"{"o":{"n":18446744073709551617,"f":1.5},"o":{"f":1.5}}"#,
                r#"{"o":{"f":1.5}}"#,
            ),
            (
                r#"{"a":[18446744073709551617,1.5],"a":[1.5,18446744073709551617]}"#,
                r#"{"a":[1.5,18446744073709551617]}"#,
            ),
        ] {
            let (value, numbers) = RawNumbers::parse(text).unwrap();
            assert_eq!(written(&value, &numbers, Style::Hash), expected, "{text}");
        }
    }

    /// The same, found in a manifest line by key, with text around the key
    /// that looks like the key, a string and a brace.
    #[test]
    fn a_line_is_read_by_its_last_meta_key_and_nothing_in_strings() {
        let line = r#"{"logical_key": "q\"} \"meta\": {\"x\": -0}", "meta": {"n": 18446744073709551617, "f": 1.5}, "size": 1, "meta": {"n": 1.8446744073709552e19, "f": 1.5, "s": "-0}"}}"#;
        let mut parsed: Value = serde_json::from_str(line).unwrap();
        let mut meta = parsed["meta"].take();
        let numbers = RawNumbers::read(&mut meta, line, "meta");
        assert!(numbers.is_empty(), "{numbers:?}");

        let line = r#"{"logical_key": "q\"} \"meta\": {\"x\": -0}", "meta": {"f": 1.5}, "size": 1, "meta": {"n": 18446744073709551617, "f": 1.5, "s": "-0}"}}"#;
        let mut parsed: Value = serde_json::from_str(line).unwrap();
        let mut meta = parsed["meta"].take();
        let numbers = RawNumbers::read(&mut meta, line, "meta");
        assert_eq!(
            numbers.0,
            BTreeMap::from([("/n".to_string(), "18446744073709551617".to_string())])
        );
    }

    /// A number only applies while the value is still the float it was read as.
    #[test]
    fn a_value_edited_after_reading_gets_no_old_text() {
        let (mut value, numbers) = RawNumbers::parse(r#"{"n": 18446744073709551616}"#).unwrap();
        value["n"] = serde_json::json!(3);
        assert_eq!(written(&value, &numbers, Style::Hash), r#"{"n":3}"#);
        value["n"] = serde_json::json!(1.5);
        assert_eq!(written(&value, &numbers, Style::Hash), r#"{"n":1.5}"#);
        value["n"] = serde_json::json!("text");
        assert_eq!(written(&value, &numbers, Style::Hash), r#"{"n":"text"}"#);
    }

    #[test]
    fn manifest_style_writes_floats_as_python_does_and_keeps_key_order() {
        let (value, numbers) =
            RawNumbers::parse(r#"{"z": 1e16, "a": [1e-7, 0.1, 1.0, 100000.0], "é": "é"}"#).unwrap();
        assert_eq!(
            written(&value, &numbers, Style::Manifest),
            r#"{"z":1e+16,"a":[1e-07,0.1,1.0,100000.0],"é":"é"}"#
        );
        assert_eq!(
            written(&value, &numbers, Style::Hash),
            r#"{"a":[1e-07,0.1,1.0,100000.0],"z":1e+16,"\u00e9":"\u00e9"}"#
        );
    }

    #[test]
    fn a_line_gives_the_numbers_under_its_own_key() {
        let line = r#"{"logical_key": "a", "meta": {"n": 18446744073709551616, "z": -0}, "user_meta": {"m": 36893488147419103232}}"#;
        let mut parsed: Value = serde_json::from_str(line).unwrap();
        let mut meta = parsed["meta"].take();
        let numbers = RawNumbers::read(&mut meta, line, "meta");
        assert_eq!(
            numbers.0,
            BTreeMap::from([("/n".to_string(), "18446744073709551616".to_string())])
        );
        assert_eq!(meta["z"], serde_json::json!(0));
    }

    #[test]
    fn only_without_and_union_split_and_join_by_pointer() {
        let (_, numbers) = RawNumbers::parse(
            r#"{"n": 18446744073709551616, "agent_exec": {"m": 36893488147419103232}, "ok": 1}"#,
        )
        .unwrap();
        let task = numbers.only("/agent_exec");
        let rest = numbers.without("/agent_exec");
        assert_eq!(
            task.0,
            BTreeMap::from([(
                "/agent_exec/m".to_string(),
                "36893488147419103232".to_string()
            )])
        );
        assert_eq!(
            rest.0,
            BTreeMap::from([("/n".to_string(), "18446744073709551616".to_string())])
        );
        assert_eq!(rest.union(&task), numbers);
        assert!(numbers.only("/missing").is_empty());
    }

    #[test]
    fn still_in_keeps_matching_numbers_and_drops_replaced_ones() {
        let (mut value, numbers) = RawNumbers::parse(
            r#"{"n": 18446744073709551616, "agent_exec": {"claim": {"seq": 36893488147419103232}}}"#,
        )
        .unwrap();
        assert!(!numbers.still_in(&value).only("/n").is_empty());
        assert!(
            !numbers
                .still_in(&value)
                .only("/agent_exec/claim")
                .is_empty()
        );
        value["agent_exec"]["claim"] = serde_json::json!({"status": "claimed"});
        let kept = numbers.still_in(&value);
        assert!(!kept.only("/n").is_empty());
        assert!(kept.only("/agent_exec/claim").is_empty());
    }
}

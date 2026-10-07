//! A JSON value printed on one line within a character budget.
//!
//! A port of the catalog's `utils/JSONOneliner.ts`, so a value folded here reads
//! as it does in the catalog: `{ A: 1, B: true, C, <…2> }`. What does not fit is
//! dropped from the end and counted: a key whose value is too long keeps its
//! key, and a nested value too long for the room left is folded to `<…N>`.
//!
//! The port is faithful, quirks included, and the catalog's own test cases are
//! below, so the two cannot drift without a test saying so. Sizes are in
//! characters — Unicode scalar values, where the catalog counts UTF-16 units —
//! and the budget is an `f64` because the caller divides a width by a
//! character's.

use serde_json::Value;

/// What a part is, which decides how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Brace,
    /// `: ` between a key and its value.
    Equal,
    Key,
    /// A nested array or object, expanded or folded by the second pass.
    Object,
    /// A number, a boolean or `null`.
    Primitive,
    Separator,
    String,
    /// `<…N>`: how many entries were left out.
    More,
}

/// One printed part.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub kind: Kind,
    /// The text, as printed. A string's is its JSON form, quotes included.
    pub value: String,
    /// A string's own text, without quotes or escapes, for drawing it.
    pub text: Option<String>,
    size: f64,
    /// A nested value, for the second pass.
    nested: Option<Value>,
    children: usize,
}

impl Part {
    fn new(kind: Kind, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
            text: None,
            size: len(value),
            nested: None,
            children: 0,
        }
    }
}

/// A line, and the budget it left.
#[derive(Clone, Debug, PartialEq)]
pub struct Printed {
    pub parts: Vec<Part>,
    pub available: f64,
}

#[allow(
    clippy::cast_precision_loss,
    reason = "a count of characters on one line, far below 2^52"
)]
fn len(s: &str) -> f64 {
    s.chars().count() as f64
}

fn equal() -> Part {
    Part::new(Kind::Equal, ": ")
}
fn separator() -> Part {
    Part::new(Kind::Separator, ", ")
}
fn empty() -> Part {
    Part::new(Kind::Separator, "")
}

fn braces(array: bool) -> (Part, Part) {
    if array {
        (Part::new(Kind::Brace, "[ "), Part::new(Kind::Brace, " ]"))
    } else {
        (Part::new(Kind::Brace, "{ "), Part::new(Kind::Brace, " }"))
    }
}

/// An entry of the first level: a lone part, or a key with its value.
enum Entry {
    Part(Part),
    Group { elements: Vec<Part>, size: f64 },
}

impl Entry {
    fn size(&self) -> f64 {
        match self {
            Self::Part(p) => p.size,
            Self::Group { size, .. } => *size,
        }
    }

    fn group(elements: Vec<Part>) -> Self {
        let size = elements.iter().map(|e| e.size).sum();
        Self::Group { elements, size }
    }
}

fn value_part(value: &Value) -> Part {
    match value {
        Value::String(s) => Part {
            kind: Kind::String,
            value: Value::String(s.clone()).to_string(),
            text: Some(s.clone()),
            size: len(s) + 2.0,
            nested: None,
            children: 0,
        },
        Value::Number(_) | Value::Bool(_) | Value::Null => {
            Part::new(Kind::Primitive, &value.to_string())
        }
        Value::Array(items) => nested(value, items.len(), '[', ']'),
        Value::Object(fields) => nested(value, fields.len(), '{', '}'),
    }
}

fn nested(value: &Value, children: usize, open: char, close: char) -> Part {
    let printed = format!("{open} …<{children}> {close}");
    Part {
        nested: Some(value.clone()),
        children,
        ..Part::new(Kind::Object, &printed)
    }
}

fn with_separators(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out = Vec::with_capacity(entries.len() * 2);
    for (i, entry) in entries.into_iter().enumerate() {
        if i > 0 {
            out.push(Entry::Part(separator()));
        }
        out.push(entry);
    }
    out
}

fn entries(value: &Value, show_values: bool) -> Vec<Entry> {
    let (inner, array) = match value {
        Value::Array(items) => (
            items
                .iter()
                .map(|v| {
                    if show_values {
                        Entry::group(vec![value_part(v)])
                    } else {
                        Entry::Part(empty())
                    }
                })
                .collect(),
            true,
        ),
        Value::Object(fields) => (
            fields
                .iter()
                .map(|(k, v)| {
                    let key = Part::new(Kind::Key, k);
                    if show_values {
                        Entry::group(vec![key, equal(), value_part(v)])
                    } else {
                        Entry::group(vec![key])
                    }
                })
                .collect(),
            false,
        ),
        _ => (Vec::new(), false),
    };
    let (left, right) = braces(array);
    let mut out = vec![Entry::Part(left)];
    out.extend(with_separators(inner));
    out.push(Entry::Part(right));
    out
}

fn push(mut line: Printed, part: Part) -> Printed {
    line.available -= part.size;
    line.parts.push(part);
    line
}

fn rest_size(entries: &[Entry], index: usize) -> f64 {
    entries[index..]
        .iter()
        .map(|e| match e {
            Entry::Part(p) => p.size,
            Entry::Group { elements, .. } => elements.first().map_or(0.0, |e| e.size),
        })
        .sum()
}

/// The catalog counts the entry at `index` twice — its whole size and its
/// first element's — and the port keeps it, or its outputs would differ.
fn enough_for_rest(entries: &[Entry], index: usize, available: f64) -> bool {
    available - (entries[index].size() + rest_size(entries, index)) > 0.0
}

fn first_level_more(entries: &[Entry], index: usize, single: bool) -> Part {
    let left = entries[index..]
        .iter()
        .filter(|e| matches!(e, Entry::Group { .. }))
        .count();
    if left == 0 {
        return empty();
    }
    let value = if index == 0 || single {
        format!("<…{left}>")
    } else {
        format!(", <…{left}>")
    };
    Part::new(Kind::More, &value)
}

fn enough_for_braces(entries: &[Entry], more: &Part, available: f64) -> bool {
    let last = entries.len() - 1;
    available - (entries[0].size() + entries[last].size() + more.size) > 0.0
}

fn close_first_level(line: Printed, entries: &[Entry], more: Part) -> Printed {
    let (Entry::Part(left), Entry::Part(right)) = (&entries[0], &entries[entries.len() - 1]) else {
        unreachable!("the first and last entries are braces")
    };
    let line = if line.parts.is_empty() {
        push(line, left.clone())
    } else {
        line
    };
    push(push(line, more), right.clone())
}

fn fold_second_level(mut line: Printed, part: &Part) -> Printed {
    let array = matches!(part.nested, Some(Value::Array(_)));
    let (left, right) = braces(array);
    let more = Part::new(Kind::More, &format!("<…{}>", part.children));
    line.available += part.size - left.size - more.size - right.size;
    line.parts.extend([left, more, right]);
    line
}

/// Print an array or an object on one line within `available` characters.
///
/// Anything else prints as an empty line: a scalar has nothing to fold.
#[must_use]
pub fn print(value: &Value, available: f64, show_values: bool) -> Printed {
    let entries = entries(value, show_values);
    let mut line = Printed {
        parts: Vec::new(),
        available,
    };
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Entry::Part(part) => {
                let output = push(line.clone(), part.clone());
                let more = first_level_more(&entries, index, false);
                if enough_for_braces(&entries, &more, output.available) {
                    line = output;
                } else {
                    line = close_first_level(line, &entries, more);
                    break;
                }
            }
            Entry::Group { elements, .. } => {
                if enough_for_rest(&entries, index, line.available) {
                    for element in elements {
                        line = push(line, element.clone());
                    }
                } else {
                    let output = push(line.clone(), elements[0].clone());
                    let more = first_level_more(&entries, index, true);
                    if enough_for_braces(&entries, &more, output.available) {
                        line = output;
                    } else {
                        line = close_first_level(line, &entries, more);
                        break;
                    }
                }
            }
        }
    }

    let mut out = Printed {
        parts: Vec::new(),
        available: line.available,
    };
    for part in line.parts {
        match (&part.nested, part.kind) {
            (Some(_), Kind::Object) if out.available <= 0.0 => {
                out = fold_second_level(out, &part);
            }
            (Some(nested), Kind::Object) => {
                let inner = print(nested, out.available + part.size, show_values);
                out.available = inner.available;
                out.parts.extend(inner.parts);
            }
            _ => out.parts.push(part),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    //! The catalog's `JSONOneliner.spec.ts`, case for case.

    use super::*;
    use serde_json::json;

    fn printed(value: &Value, limit: f64) -> (String, f64) {
        let p = print(value, limit, true);
        let text: String = p.parts.iter().map(|p| p.value.as_str()).collect();
        (text, p.available)
    }

    fn check(value: &Value, limit: f64, expected: &str) {
        let (text, available) = printed(value, limit);
        assert_eq!(text, expected);
        assert!(
            (available - (limit - len(expected))).abs() < f64::EPSILON,
            "{available} left for {expected:?} at {limit}"
        );
    }

    #[test]
    fn empty_brackets_for_an_empty_array() {
        check(&json!([]), 10.0, "[  ]");
    }

    #[test]
    fn empty_brackets_for_an_empty_object() {
        check(&json!({}), 10.0, "{  }");
    }

    #[test]
    fn every_primitive_in_an_array_when_there_is_room() {
        check(
            &json!([1, true, false, "Lorem", null]),
            100.0,
            r#"[ 1, true, false, "Lorem", null ]"#,
        );
    }

    #[test]
    fn every_primitive_in_an_object_when_there_is_room() {
        check(
            &json!({"A": 1, "B": true, "C": false, "D": "Lorem", "E": null}),
            100.0,
            r#"{ A: 1, B: true, C: false, D: "Lorem", E: null }"#,
        );
    }

    #[test]
    fn placeholders_in_an_array_when_there_is_not() {
        check(
            &json!([1, true, false, "Lorem", null]),
            30.0,
            "[ 1, true, false, <…2> ]",
        );
    }

    #[test]
    fn placeholders_in_an_object_when_there_is_not() {
        check(
            &json!({"A": 1, "B": true, "C": false, "D": "Lorem", "E": null}),
            30.0,
            "{ A: 1, B: true, C, <…2> }",
        );
    }

    #[test]
    fn a_key_alone_when_its_value_is_too_long() {
        check(
            &json!({
                "A": 1,
                "D": "https://ru.wikipedia.org/wiki/%D0%97%D0%B0%D0%B3%D0%BB%D0%B0%D0%B2%D0%BD%D0%B0%D1%8F_%D1%81%D1%82%D1%80%D0%B0%D0%BD%D0%B8%D1%86%D0%B0",
                "B": true,
                "C": false,
                "E": null,
            }),
            30.0,
            "{ A: 1, D, B: true, <…2> }",
        );
    }

    #[test]
    fn every_nested_array_value_when_there_is_room() {
        check(
            &json!([[1, 2, 3], "Lorem", ["a", "b"]]),
            100.0,
            r#"[ [ 1, 2, 3 ], "Lorem", [ "a", "b" ] ]"#,
        );
    }

    fn nested_fixture() -> Value {
        json!({
            "A": [1, 2, 3],
            "B": "Lorem",
            "C": {"a": 1, "b": 2},
            "D": {"a": [1, {"c": 3}, "a"], "b": 2},
            "E": [1, {"b": ["c", {"d": "e"}]}, "a"],
        })
    }

    #[test]
    fn every_nested_object_value_when_there_is_room() {
        check(
            &nested_fixture(),
            140.0,
            r#"{ A: [ 1, 2, 3 ], B: "Lorem", C: { a: 1, b: 2 }, D: { a: [ 1, { c: 3 }, "a" ], b: 2 }, E: [ 1, { b: [ "c", { d: "e" } ] }, "a" ] }"#,
        );
    }

    #[test]
    fn placeholders_for_nested_values_as_the_room_shrinks() {
        let v = nested_fixture();
        check(
            &v,
            130.0,
            r#"{ A: [ 1, 2, 3 ], B: "Lorem", C: { a: 1, b: 2 }, D: { a: [ 1, { c: 3 }, "a" ], b: 2 }, E: [ 1, { b: [ "c", <…1> ] }, "a" ] }"#,
        );
        check(
            &v,
            120.0,
            r#"{ A: [ 1, 2, 3 ], B: "Lorem", C: { a: 1, b: 2 }, D: { a: [ 1, { c: 3 }, "a" ], b: 2 }, E: [ 1, { b }, "a" ] }"#,
        );
        check(
            &v,
            110.0,
            r#"{ A: [ 1, 2, 3 ], B: "Lorem", C: { a: 1, b: 2 }, D: { a: [ 1, { c: 3 }, "a" ], b: 2 }, E: [ 1, <…2> ] }"#,
        );
        check(
            &v,
            50.0,
            r#"{ A: [ 1, <…2> ], B: "Lorem", C: { <…2> }, <…2> }"#,
        );
    }

    #[test]
    fn keys_only_when_values_are_hidden() {
        let p = print(&json!({"a": 1, "b": [2]}), 100.0, false);
        let text: String = p.parts.iter().map(|p| p.value.as_str()).collect();
        assert_eq!(text, "{ a, b }");
    }
}

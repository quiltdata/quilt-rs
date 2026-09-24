//! A command's error text, as a reader should see it.

/// The words a reader should see for a command's error text.
///
/// `Error::to_frontend_string` sends some errors as an [`ErrorResponse`] JSON —
/// `{"kind":…,"message":…}` — so a caller can route on `kind`. Drawn as it
/// arrives, that is the envelope, not the message. This returns the `message`
/// of such a JSON and any other text unchanged.
///
/// Apply it where the text is drawn, never where it is received: whatever routes
/// on `kind`, as `error_handler::handle_or_display` does, needs the raw string.
#[must_use]
pub fn readable(error: &str) -> String {
    // An object first: serde would also read `ErrorResponse` from a two-item array.
    serde_json::from_str::<serde_json::Value>(error)
        .ok()
        .filter(serde_json::Value::is_object)
        .and_then(|value| serde_json::from_value::<ErrorResponse>(value).ok())
        .map_or_else(|| error.to_string(), |e| e.message)
}

/// The envelope `Error::to_frontend_string` sends. Also what
/// `error_handler` routes on, so the two read one shape.
#[derive(serde::Deserialize)]
pub(crate) struct ErrorResponse {
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_error_reads_as_its_message() {
        assert_eq!(
            readable(r#"{"kind":"access_denied","message":"No access.","host":"example.com"}"#),
            "No access."
        );
    }

    #[test]
    fn plain_text_reads_as_itself() {
        assert_eq!(readable("connection reset"), "connection reset");
        assert_eq!(readable(""), "");
    }

    #[test]
    fn json_of_another_shape_reads_as_itself() {
        for text in [
            r#"{"kind":"access_denied"}"#,
            r#"{"message":"No kind."}"#,
            r#"{"kind":"not_found","message":7}"#,
            r#"["access_denied","No access."]"#,
            r#""No access.""#,
        ] {
            assert_eq!(readable(text), text);
        }
    }
}

use crate::cli::Error;
use std::io::Write;

/// How a command's result is rendered. Decided once from the global `--json`
/// flag and handed to [`print`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

/// A command output that can render itself for a person or for a program.
///
/// The `to_json` default is transitional. It is removed once every command has
/// a real payload, and from then on an output without one does not compile —
/// which is what keeps `--json` universal without a checklist to maintain.
pub trait Render: std::fmt::Display {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "message": self.to_string() })
    }
}

pub enum Std {
    Out(Box<dyn Render>),
    Err(Error),
}

/// Hand-written rather than derived: `Box<dyn Render>` carries no `Debug`, and
/// requiring one on the trait would force a derive onto every `Output` and the
/// library types they hold. The JSON is the more useful thing to see in a
/// failing test anyway — it is what is under test.
impl std::fmt::Debug for Std {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Std::Out(rendered) => write!(f, "Out({:?})", rendered.to_json()),
            Std::Err(err) => write!(f, "Err({err})"),
        }
    }
}

impl Std {
    pub fn from_result<T: Render + 'static>(result: Result<T, Error>) -> Self {
        match result {
            Ok(r) => Std::Out(Box::new(r)),
            Err(err) => Std::Err(err),
        }
    }
}

fn error_json(err: &Error) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "kind": err.kind(),
            "message": err.to_string(),
        }
    })
}

pub fn print(
    output: Std,
    format: Format,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), std::io::Error> {
    match (output, format) {
        (Std::Out(rendered), Format::Text) => writeln!(stdout, "{rendered}")?,
        (Std::Out(rendered), Format::Json) => writeln!(stdout, "{}", rendered.to_json())?,
        (Std::Err(err), Format::Text) => writeln!(stderr, "{err}")?,
        (Std::Err(err), Format::Json) => writeln!(stderr, "{}", error_json(&err))?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for a command output, so these tests pin `print`'s routing
    /// and not any one command's payload.
    struct Fake;

    impl std::fmt::Display for Fake {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "human text")
        }
    }

    impl Render for Fake {
        fn to_json(&self) -> serde_json::Value {
            serde_json::json!({ "machine": true })
        }
    }

    #[test]
    fn json_success_goes_to_stdout_and_leaves_stderr_empty() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        print(
            Std::Out(Box::new(Fake)),
            Format::Json,
            &mut stdout,
            &mut stderr,
        )
        .expect("write succeeds");

        assert!(stderr.is_empty(), "stderr must stay clean on success");
        let parsed: serde_json::Value =
            serde_json::from_slice(&stdout).expect("stdout is valid JSON");
        assert_eq!(parsed["machine"], true);
    }

    /// The property the whole bare-object-on-stdout decision rests on: a
    /// consumer parsing stdout gets nothing at all rather than half an object.
    #[test]
    fn json_error_goes_to_stderr_and_leaves_stdout_empty() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        print(
            Std::Err(Error::NamespaceRequired),
            Format::Json,
            &mut stdout,
            &mut stderr,
        )
        .expect("write succeeds");

        assert!(stdout.is_empty(), "stdout must stay empty on failure");
        let parsed: serde_json::Value =
            serde_json::from_slice(&stderr).expect("stderr is valid JSON");
        assert_eq!(parsed["error"]["kind"], "namespace_required");
        assert!(
            parsed["error"]["message"]
                .as_str()
                .expect("message is a string")
                .contains("Could not infer a namespace"),
        );
    }

    #[test]
    fn text_format_still_prints_display() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        print(
            Std::Out(Box::new(Fake)),
            Format::Text,
            &mut stdout,
            &mut stderr,
        )
        .expect("write succeeds");

        assert_eq!(String::from_utf8(stdout).expect("utf8"), "human text\n");
        assert!(stderr.is_empty());
    }
}

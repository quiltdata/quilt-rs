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
/// `to_json` has no default on purpose: an output that cannot describe itself
/// to a machine does not compile, which is what keeps `--json` universal
/// without a checklist to maintain.
pub trait Render: std::fmt::Display {
    fn to_json(&self) -> serde_json::Value;
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

    /// [`Render::to_json`] returns `serde_json::Value`, not a bare-object type,
    /// so nothing at the type level stops a future command from returning
    /// `json!([…])` or `json!(null)`. The spec and the README promise an
    /// object; this pins that promise against a representative sample of the
    /// real `Output` types (spanning a bare hash, a hash-plus-bool-plus-uri
    /// shape, and a nested-report shape) rather than changing the trait's
    /// return type, which would force a signature change onto every impl for
    /// a guarantee one test can buy.
    #[test]
    fn to_json_is_always_a_bare_object() {
        use crate::cli::commit;
        use crate::cli::pull;
        use crate::cli::push;
        use crate::cli::status;
        use crate::cli::undo_commit;
        use quilt_rs::lineage::CommitState;
        use quilt_rs::lineage::InstalledPackageStatus;

        let commit_out = commit::Output {
            commit: CommitState::default(),
        };
        assert!(commit_out.to_json().is_object());

        let undo_commit_out = undo_commit::Output {
            commit: CommitState::default(),
        };
        assert!(undo_commit_out.to_json().is_object());

        let manifest_uri = quilt_uri::ManifestUri {
            origin: None,
            bucket: "bucket".to_string(),
            namespace: ("demo", "sales").into(),
            hash: "abc123".to_string(),
        };
        let push_out = push::Output {
            hash: "abc123".to_string(),
            manifest_uri: manifest_uri.clone(),
            certified_latest: true,
        };
        assert!(push_out.to_json().is_object());

        let pull_out = pull::Output {
            hash: "abc123".to_string(),
            report: quilt_rs::flow::PullReport {
                manifest_uri,
                added: Vec::new(),
                added_not_fetched: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                message: None,
            },
        };
        assert!(pull_out.to_json().is_object());

        let status_out = status::Output {
            status: InstalledPackageStatus::default(),
        };
        assert!(status_out.to_json().is_object());
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

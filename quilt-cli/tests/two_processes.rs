//! Two `quilt` processes on one package, as from two shells, run the real
//! binary against one domain.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::process::Output;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

mod common;

use quilt_rs::paths::DomainPaths;
use quilt_uri::Namespace;

/// The line `quilt` prints to stderr when it waits for the package's lock.
const WAITING_NOTICE: &str = "waiting for another quilt process…";

const DEADLINE: Duration = Duration::from_secs(20);

/// Runs `quilt` on `domain` to the end and returns its output if it succeeded.
fn quilt(domain: &Path, args: &[&str]) -> Output {
    let output = common::quilt_command(domain)
        .args(args)
        .output()
        .expect("quilt runs");
    assert!(
        output.status.success(),
        "quilt {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A `quilt` process still running, with its stderr gathered as it comes.
/// Dropping it kills the process, so a failed assertion leaves none behind.
struct Running {
    child: Child,
    stderr: Arc<Mutex<String>>,
    reader: Option<JoinHandle<()>>,
}

impl Running {
    fn spawn(domain: &Path, args: &[&str]) -> Self {
        let mut child = common::quilt_command(domain)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("quilt starts");
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut pipe = child.stderr.take().expect("stderr is piped");
        let reader = std::thread::spawn({
            let stderr = Arc::clone(&stderr);
            move || {
                let mut buf = [0; 1024];
                while let Ok(n @ 1..) = pipe.read(&mut buf) {
                    stderr
                        .lock()
                        .expect("stderr")
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
        });
        Self {
            child,
            stderr,
            reader: Some(reader),
        }
    }

    fn stderr(&self) -> String {
        self.stderr.lock().expect("stderr").clone()
    }

    fn is_running(&mut self) -> bool {
        self.child.try_wait().expect("quilt's status").is_none()
    }

    /// Waits for the notice on stderr, failing at the deadline.
    fn wait_for_notice(&self) {
        let deadline = Instant::now() + DEADLINE;
        while !self.stderr().contains(WAITING_NOTICE) {
            assert!(
                Instant::now() < deadline,
                "no notice; stderr: {}",
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for the process to exit 0, failing at the deadline, and returns
    /// its stdout and all of its stderr.
    fn finish(mut self) -> (String, String) {
        let deadline = Instant::now() + DEADLINE;
        while self.is_running() {
            assert!(Instant::now() < deadline, "quilt never exited");
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = self.child.wait().expect("quilt's status");
        assert!(status.success(), "quilt failed: {}", self.stderr());
        let mut stdout = String::new();
        self.child
            .stdout
            .take()
            .expect("stdout is piped")
            .read_to_string(&mut stdout)
            .expect("stdout");
        if let Some(reader) = self.reader.take() {
            reader.join().expect("stderr is read");
        }
        (stdout, self.stderr())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn commit_hash(stdout: &str) -> String {
    let output: serde_json::Value = serde_json::from_str(stdout).expect("commit's JSON");
    output["hash"].as_str().expect("a commit hash").to_string()
}

/// Two commits from two processes, started while a third holds the package's
/// lock: both wait and say so, then run one after the other, and the second
/// builds on the first. Without the lock both would build on the created
/// revision, and the later write would drop the other's commit.
#[test]
fn two_processes_committing_one_package_keep_both_commits() {
    let dir = tempfile::TempDir::new().expect("a temporary folder");
    let domain = dir.path().join("domain");
    let home = dir.path().join("home");
    quilt(&domain, &["home", home.to_str().expect("a UTF-8 path")]);
    quilt(&domain, &["create", "--namespace", "test/pkg"]);

    let paths = DomainPaths::new(domain.clone());
    let namespace: Namespace = ("test", "pkg").into();
    let lock_path = paths.package_lock(&namespace);
    fs::create_dir_all(lock_path.parent().expect("the locks folder")).expect("the locks folder");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .expect("the lock file");
    lock_file.lock().expect("the package's lock");

    let commit = |message| {
        Running::spawn(
            &domain,
            &["--json", "commit", "-m", message, "--namespace", "test/pkg"],
        )
    };
    let mut one = commit("one");
    let mut two = commit("two");
    one.wait_for_notice();
    two.wait_for_notice();
    assert!(one.is_running(), "the first commit waits for the lock");
    assert!(two.is_running(), "the second commit waits for the lock");

    drop(lock_file);
    let mut messages = std::collections::HashMap::new();
    for (writer, message) in [(one, "one"), (two, "two")] {
        let (stdout, stderr) = writer.finish();
        assert_eq!(stderr.matches(WAITING_NOTICE).count(), 1, "said once");
        messages.insert(commit_hash(&stdout), message);
    }

    let lineage: serde_json::Value =
        serde_json::from_slice(&fs::read(paths.lineage()).expect("data.json")).expect("its JSON");
    let head = &lineage["packages"]["test/pkg"]["commit"];
    let last = head["hash"].as_str().expect("the head's hash");
    let first = head["prev_hashes"][0].as_str().expect("the head's parent");
    assert!(
        messages.contains_key(last) && messages.contains_key(first) && first != last,
        "the last commit builds on the first: {head}"
    );

    let log: serde_json::Value = serde_json::from_slice(
        &quilt(&domain, &["--json", "log", "--namespace", "test/pkg"]).stdout,
    )
    .expect("log's JSON");
    let history: Vec<&str> = log["revisions"]
        .as_array()
        .expect("revisions")
        .iter()
        .map(|revision| revision["message"].as_str().expect("a message"))
        .collect();
    assert_eq!(
        history,
        [messages[last], messages[first], "Created package"],
        "the history keeps both commits"
    );
}

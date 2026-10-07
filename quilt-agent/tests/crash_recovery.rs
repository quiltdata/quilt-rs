//! A run interrupted between upload and publish resumes from the journal and
//! lands exactly once: one sentinel, one revision, `latest` set once (INV-5).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use quilt_agent::agent::Agent;
use quilt_agent::profile::Profile;
use quilt_agent::spool::RunState;
use quilt_agent::spool::Spool;
use quilt_rs::Res;
use quilt_rs::io::remote::HostConfig;
use quilt_rs::io::remote::PutCondition;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteObjectStream;
use quilt_rs::io::remote::mocks::MockRemote;
use quilt_rs::io::storage::ByteStream;
use quilt_rs::object_hash::ObjectHash;
use quilt_rs::object_hash::Sha256ChunkedHash;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

/// The mock remote, with a switch that makes the next sentinel write fail —
/// standing in for the process dying after its uploads, before its seal.
struct Flaky {
    inner: MockRemote,
    fail_seal: AtomicBool,
}

impl Remote for Flaky {
    async fn exists(&self, h: Option<&Host>, u: &S3Uri) -> Res<bool> {
        self.inner.exists(h, u).await
    }
    async fn get_object_stream(&self, h: Option<&Host>, u: &S3Uri) -> Res<RemoteObjectStream> {
        self.inner.get_object_stream(h, u).await
    }
    async fn resolve_url(&self, h: Option<&Host>, u: &S3Uri) -> Res<S3Uri> {
        self.inner.resolve_url(h, u).await
    }
    async fn put_object(&self, h: Option<&Host>, u: &S3Uri, c: impl Into<ByteStream>) -> Res {
        self.inner.put_object(h, u, c).await
    }
    async fn put_object_if(
        &self,
        h: Option<&Host>,
        u: &S3Uri,
        c: impl Into<ByteStream>,
        cond: PutCondition,
    ) -> Res {
        if u.key.ends_with(".quilt-sentinel.json") && self.fail_seal.swap(false, Ordering::SeqCst) {
            return Err(quilt_rs::Error::Io(std::io::Error::other(
                "network dropped",
            )));
        }
        self.inner.put_object_if(h, u, c, cond).await
    }
    async fn get_object_etag(&self, h: Option<&Host>, u: &S3Uri) -> Res<Option<String>> {
        self.inner.get_object_etag(h, u).await
    }
    async fn upload_file(
        &self,
        _c: &HostConfig,
        p: impl AsRef<Path>,
        u: &S3Uri,
        s: u64,
    ) -> Res<(S3Uri, ObjectHash)> {
        // Store the bytes (the mock's own upload only hashes, from its private
        // tempdir) so the landing reads back like a bucket.
        let bytes = std::fs::read(p.as_ref())?;
        let hash = Sha256ChunkedHash::from_async_read(bytes.as_slice(), s).await?;
        self.inner.put_object(None, u, bytes).await?;
        Ok((
            S3Uri {
                version: Some("v1".into()),
                ..u.clone()
            },
            hash.into(),
        ))
    }
    async fn host_config(&self, h: Option<&Host>) -> Res<HostConfig> {
        self.inner.host_config(h).await
    }
    async fn verify_bucket(&self, b: &str) -> Res {
        self.inner.verify_bucket(b).await
    }
    async fn published_revisions(&self, h: &Host, b: &str, n: &Namespace) -> Res<Vec<String>> {
        self.inner.published_revisions(h, b, n).await
    }
}

fn profile(source: &Path) -> Profile {
    Profile::parse(&format!(
        r#"
schema_version: "1"
observer: {{ id: edge-01, placement: beside }}
registry: {{ url: "https://example.quiltdata.com", credential_ref: k }}
instruments:
  - id: reader-1
    source: {{ path: "{}" }}
    landing: {{ bucket: raw, prefix: lab }}
    boundary: {{ method: marker_file, markers: ["done.txt"], confirm_window_s: 60 }}
"#,
        source.display()
    ))
    .expect("test profile is valid")
}

async fn read(remote: &impl Remote, key: &str) -> String {
    let uri = S3Uri {
        bucket: "raw".into(),
        key: key.into(),
        version: None,
    };
    let bytes = remote
        .get_object_stream(None, &uri)
        .await
        .expect("object exists")
        .body
        .collect()
        .await
        .expect("readable")
        .into_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf8")
}

#[tokio::test]
async fn interrupted_run_lands_exactly_once() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let source = tempfile::tempdir()?;
    let spool_dir = tempfile::tempdir()?;
    let run = source.path().join("2026-10-07_plate1");
    std::fs::create_dir(&run)?;
    std::fs::write(run.join("a.fcs"), b"aaaa")?;
    std::fs::write(run.join("b.fcs"), b"bbbbbb")?;
    std::fs::write(run.join("done.txt"), b"")?;
    let later = SystemTime::now() + Duration::from_secs(3600);

    // Pass 1 decides the boundary, snapshots, uploads, then dies at the seal.
    let flaky = Flaky {
        inner: MockRemote::default(),
        fail_seal: AtomicBool::new(true),
    };
    let mut first = Agent::new(
        profile(source.path()),
        flaky,
        Spool::open(spool_dir.path())?,
        None,
    );
    first.pass(SystemTime::now()).await?; // first sight: listing recorded
    first.pass(later).await?; // quiet long enough: complete, upload, seal fails
    let runs = first.spool.runs()?;
    let (run_id, r) = runs.iter().next().expect("one run journalled");
    assert_eq!(r.state, RunState::Uploading);
    assert_eq!(r.verified.len(), 3, "all members verified before the seal");

    // Pass 2 is a new process: same spool, same bucket, nothing in memory.
    let Agent { remote, .. } = first;
    let mut second = Agent::new(
        profile(source.path()),
        remote,
        Spool::open(spool_dir.path())?,
        None,
    );
    second.pass(later).await?;
    second.pass(later).await?; // a further pass must not land it again

    let runs = second.spool.runs()?;
    assert_eq!(runs.len(), 1, "the folder was not observed as a second run");
    assert_eq!(runs[run_id].state, RunState::Landed);

    let sentinel: serde_json::Value = serde_json::from_str(
        &read(
            &second.remote,
            &format!("lab/reader-1/{run_id}/.quilt-sentinel.json"),
        )
        .await,
    )?;
    quilt_agent::sentinel::validate(&sentinel)?;
    assert_eq!(sentinel["file_count"], 3);
    assert_eq!(sentinel["boundary"]["guessed"], false);

    let latest = read(
        &second.remote,
        ".quilt/named_packages/reader-1/2026-10-07_plate1/latest",
    )
    .await;
    let journal = std::fs::read_to_string(spool_dir.path().join("journal.jsonl"))?;
    assert_eq!(journal.matches(r#""event":"landed""#).count(), 1);
    assert!(
        journal.contains(&latest),
        "the landed hash is the one latest names"
    );
    Ok(())
}

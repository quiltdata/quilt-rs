//! One pass of the agent over every instrument: find run folders, decide their
//! boundaries, freeze membership, upload, seal, publish. Each step is journalled
//! before the next starts, so a pass after a crash picks up where the journal
//! ends and does nothing twice that matters.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::Utc;
use quilt_rs::flow;
use quilt_rs::io::remote::HostConfig;
use quilt_rs::io::remote::PutCondition;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::storage::LocalStorage;
use quilt_uri::Host;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3Uri;
use serde_json::json;

use crate::Error;
use crate::boundary;
use crate::boundary::Verdict;
use crate::packager;
use crate::profile::Instrument;
use crate::profile::Method;
use crate::profile::Profile;
use crate::sentinel::Checksum;
use crate::sentinel::FileEntry;
use crate::sentinel::SENTINEL_NAME;
use crate::sentinel::Sentinel;
use crate::spool::Event;
use crate::spool::RunState;
use crate::spool::SnapMember;
use crate::spool::Spool;
use crate::watch;
use crate::watch::Tracked;

pub struct Agent<R> {
    pub profile: Profile,
    pub remote: R,
    pub spool: Spool,
    /// `None` publishes on ambient AWS credentials (bucket-only mode).
    pub host: Option<Host>,
    tracked: BTreeMap<PathBuf, Tracked>,
    /// (folder, member count) already warned about, so a warning fires per change.
    late: std::collections::BTreeSet<(PathBuf, usize)>,
}

impl<R: Remote + Sync> Agent<R> {
    pub fn new(profile: Profile, remote: R, spool: Spool, host: Option<Host>) -> Self {
        Agent {
            profile,
            remote,
            spool,
            host,
            tracked: BTreeMap::new(),
            late: std::collections::BTreeSet::new(),
        }
    }

    /// Scan every instrument and push every run as far as it can go now.
    pub async fn pass(&mut self, now: SystemTime) -> Result<(), Error> {
        for instrument in self.profile.instruments.clone() {
            if let Err(e) = self.observe(&instrument, now) {
                // One unreadable share must not stall the other instruments.
                tracing::warn!(instrument = instrument.id, "scan failed: {e}");
            }
        }
        self.resume().await
    }

    /// Finish every journalled run that has not landed.
    pub async fn resume(&mut self) -> Result<(), Error> {
        for (run_id, run) in self.spool.runs()? {
            let result = match run.state {
                RunState::Uploading | RunState::Sealed => self.finish(&run_id).await,
                RunState::Landed | RunState::Refused | RunState::Suspect => Ok(()),
            };
            match result {
                Ok(()) => {}
                // Not transient: park the run with its reason and move on.
                Err(Error::Refused(reason)) => {
                    tracing::warn!(run_id, "parked: {reason}");
                    self.spool.record(&Event::Suspect { run_id, reason })?;
                }
                // Transient (network, throttling): the next pass retries.
                Err(e) => tracing::warn!(run_id, "will retry: {e}"),
            }
        }
        Ok(())
    }

    fn observe(&mut self, instrument: &Instrument, now: SystemTime) -> Result<(), Error> {
        let known = self.spool.known_folders()?;
        let ignore = watch::glob_set(&instrument.source.ignore)?;
        for folder in watch::run_folders(&instrument.source)? {
            if let Some(captured) = known.get(&folder) {
                let now: Vec<SnapMember> = watch::members(&folder, &ignore)?
                    .iter()
                    .map(snap_member)
                    .collect();
                if captured.iter().all(|m| now.contains(m)) {
                    // A file that appears after the boundary is a new
                    // observation, never a late member. Say so once per
                    // change instead of dropping it silently.
                    if now.len() > captured.len() && self.late.insert((folder.clone(), now.len())) {
                        tracing::warn!(
                            folder = %folder.display(),
                            captured = captured.len(),
                            now = now.len(),
                            "files appeared after this run was captured; they are not in its revision"
                        );
                    }
                    continue;
                }
                // A captured member is gone or rewritten: the instrument wrote
                // a new run at the same path. Observe it as one.
            }
            let members = watch::members(&folder, &ignore)?;
            let tracked = Tracked::update(self.tracked.remove(&folder), members, now);
            let verdict = boundary::decide(
                &instrument.boundary,
                instrument.quiet_window_s(),
                &boundary::Observation {
                    members: &tracked.members,
                    requested_at: completion_request(instrument, &folder),
                    last_change: tracked.last_change,
                    now,
                },
            );
            match verdict {
                Verdict::Pending => {
                    self.tracked.insert(folder, tracked);
                }
                Verdict::Suspect(reason) => {
                    tracing::info!(folder = %folder.display(), "suspect: {reason}");
                    self.tracked.insert(folder, tracked);
                }
                Verdict::Complete {
                    evidence,
                    instrument_mtime,
                } => self.snapshot(
                    instrument,
                    &folder,
                    &tracked,
                    &evidence,
                    instrument_mtime,
                    now,
                )?,
            }
        }
        Ok(())
    }

    fn snapshot(
        &mut self,
        instrument: &Instrument,
        folder: &Path,
        tracked: &Tracked,
        evidence: &str,
        instrument_mtime: Option<SystemTime>,
        now: SystemTime,
    ) -> Result<(), Error> {
        let now_utc: DateTime<Utc> = now.into();
        let uuid = uuid::Uuid::new_v4();
        let run_id = format!(
            "{}-{}",
            now_utc.format("%Y%m%dT%H%M%SZ"),
            &uuid.simple().to_string()[..8]
        );
        let total_bytes: u64 = tracked.members.iter().map(|m| m.size).sum();
        let b = &instrument.boundary;
        let params = match b.method {
            Method::SizeStable => json!({ "stable_for_s": instrument.quiet_window_s() }),
            _ => json!({ "markers": b.markers, "confirm_window_s": instrument.quiet_window_s() }),
        };
        self.spool.record(&Event::Snapshot {
            run_id: run_id.clone(),
            instrument_id: instrument.id.clone(),
            folder: folder.to_path_buf(),
            sentinel_id: uuid.to_string(),
            boundary: json!({
                "method": b.method.as_str(),
                "evidence": evidence,
                "guessed": b.method == Method::SizeStable,
                "decided_at_utc": now_utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "params": params,
            }),
            instrument_local_time: instrument_mtime.map(local_time),
            snapshot_time_utc: now_utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            total_bytes,
            members: tracked.members.iter().map(snap_member).collect(),
        })?;
        // Recorded after the snapshot, so the folder is known and is refused
        // once, not again on every pass.
        if total_bytes > self.profile.spool.max_bytes {
            self.spool.record(&Event::Refused {
                run_id,
                reason: format!("over_cap: {total_bytes} bytes > spool.max_bytes"),
            })?;
        }
        Ok(())
    }

    /// Drive one journalled run to `Landed`.
    async fn finish(&mut self, run_id: &str) -> Result<(), Error> {
        let runs = self.spool.runs()?;
        let run = &runs[run_id];
        let Event::Snapshot {
            instrument_id,
            folder,
            ..
        } = &run.snapshot
        else {
            unreachable!("a run is keyed by its snapshot")
        };
        let instrument = self
            .profile
            .instruments
            .iter()
            .find(|i| &i.id == instrument_id)
            .ok_or_else(|| Error::Refused(format!("instrument {instrument_id} left the profile")))?
            .clone();
        let sentinel_key = if let Some((key, _)) = &run.sealed {
            key.clone()
        } else {
            self.seal(run_id, run, &instrument).await?
        };
        self.publish(run_id, folder, &instrument, &sentinel_key)
            .await
    }

    /// Upload every member not yet verified, then write the sentinel last.
    async fn seal(
        &mut self,
        run_id: &str,
        run: &crate::spool::Run,
        instrument: &Instrument,
    ) -> Result<String, Error> {
        let Event::Snapshot {
            instrument_id,
            folder,
            sentinel_id,
            boundary,
            instrument_local_time,
            snapshot_time_utc,
            members,
            ..
        } = &run.snapshot
        else {
            unreachable!("a run is keyed by its snapshot")
        };
        let bucket = &instrument.landing.bucket;
        let run_prefix = join_key(&[&instrument.landing.prefix, &instrument.id, run_id]);
        let sentinel_key = join_key(&[&run_prefix, SENTINEL_NAME]);
        let host_config = HostConfig {
            host: self.host.clone(),
            ..HostConfig::default()
        };

        let mut files = Vec::with_capacity(members.len());
        for m in members {
            if let Some(done) = run.verified.get(&m.path) {
                files.push(done.clone());
                continue;
            }
            let file = self
                .upload(&host_config, folder, bucket, &run_prefix, m)
                .await?;
            self.spool.record(&Event::MemberVerified {
                run_id: run_id.to_string(),
                file: file.clone(),
            })?;
            files.push(file);
        }

        let previous = self.spool.previous_sentinel(instrument_id)?;
        let sentinel = Sentinel {
            schema_version: "1".into(),
            sentinel_id: sentinel_id.clone(),
            run_id: run_id.to_string(),
            previous_sentinel_id: previous.clone(),
            observer: json!({
                "id": self.profile.observer.id,
                "host": hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().into_owned()),
                "agent_version": env!("CARGO_PKG_VERSION"),
                "placement": self.profile.observer.placement,
            }),
            instrument: instrument_json(instrument),
            source_path: folder.display().to_string(),
            boundary: boundary.clone(),
            instrument_local_time: instrument_local_time.clone(),
            observer_time_utc: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            snapshot_time_utc: snapshot_time_utc.clone(),
            checksum_algorithm: files.first().map_or_else(
                || "sha2-256-chunked".into(),
                |f| f.checksum.algorithm.clone(),
            ),
            file_count: files.len(),
            total_bytes: files.iter().map(|f| f.size).sum(),
            files,
            anchor: None,
            notes: None,
        };
        let bytes = sentinel.to_validated_bytes()?;
        let uri = s3(bucket, &sentinel_key);
        if let Err(e) = self
            .remote
            .put_object_if(self.host.as_ref(), &uri, bytes, PutCondition::Absent)
            .await
        {
            // A 412 means an earlier pass sealed it and died before
            // journalling; the sentinel already in S3 is the one to keep.
            if !e.is_precondition_failed() {
                return Err(e.into());
            }
        }
        self.spool.record(&Event::Sealed {
            run_id: run_id.to_string(),
            sentinel_key: sentinel_key.clone(),
            previous_sentinel_id: previous,
        })?;
        Ok(sentinel_key)
    }

    /// Publish the sealed run as a fresh revision.
    async fn publish(
        &mut self,
        run_id: &str,
        folder: &Path,
        instrument: &Instrument,
        sentinel_key: &str,
    ) -> Result<(), Error> {
        let bucket = &instrument.landing.bucket;
        // Read the sentinel back from S3, not from memory: after a crash
        // between seal and publish it is the only copy.
        let sentinel: Sentinel = self.read_json(bucket, sentinel_key).await?;
        let crate_json = match sentinel
            .files
            .iter()
            .find(|f| f.path == packager::CRATE_NAME)
        {
            Some(f) => Some(self.read_json(bucket, &f.key).await?),
            None => None,
        };
        let folder_name = folder
            .file_name()
            .map_or_else(|| run_id.to_string(), |n| n.to_string_lossy().into_owned());
        let revision = packager::build(
            &sentinel,
            sentinel_key,
            bucket,
            &folder_name,
            &instrument.packaging.package_name_pattern,
            &self.profile.packager.eln_keys,
            crate_json.as_ref(),
        )?;
        let package = S3PackageHandle {
            bucket: bucket.clone(),
            namespace: revision.package_name.as_str().try_into().map_err(|e| {
                Error::Refused(format!("package name {}: {e}", revision.package_name))
            })?,
        };
        // A fresh revision per run; latest moves only if nobody else
        // has moved it. Re-running after a crash yields the same hash.
        // The parent is this agent's newest earlier-captured revision of the
        // same package, so a later run moves latest past it and an older run
        // retried late never does; anyone else's latest is left alone.
        let parent = self
            .spool
            .parent_for(&bucket_package_key(bucket, &revision.package_name), run_id)?;
        // A time fixed by the run, not the clock, so a replay after a crash
        // rewrites the same history entry instead of adding a second one.
        let timestamp = DateTime::parse_from_rfc3339(&sentinel.snapshot_time_utc)
            .map_err(|e| Error::Refused(format!("sentinel snapshot time: {e}")))?
            .with_timezone(&Utc);
        let pushed = flow::push_revision(
            &LocalStorage::default(),
            &self.remote,
            self.host.as_ref(),
            &package,
            revision.manifest,
            parent.as_deref(),
            timestamp,
        )
        .await?;
        tracing::info!(
            run_id,
            package = revision.package_name,
            top_hash = pushed.top_hash,
            latest_advanced = pushed.latest_advanced,
            "landed"
        );
        self.spool.record(&Event::Landed {
            run_id: run_id.to_string(),
            package_name: bucket_package_key(bucket, &revision.package_name),
            top_hash: pushed.top_hash,
            latest_advanced: pushed.latest_advanced,
        })?;
        Ok(())
    }

    async fn upload(
        &self,
        host_config: &HostConfig,
        folder: &Path,
        bucket: &str,
        run_prefix: &str,
        m: &SnapMember,
    ) -> Result<FileEntry, Error> {
        let source = folder.join(&m.path);
        // Checked before and after the upload: bytes that changed in between
        // are not the run the boundary decided on.
        Self::unchanged(&source, m)?;
        let key = join_key(&[run_prefix, &m.path]);
        let (uri, hash) = self
            .remote
            .upload_file(host_config, &source, &s3(bucket, &key), m.size)
            .await?;
        Self::unchanged(&source, m)?;
        // ObjectHash serializes as {"type", "value"}, the sentinel's checksum shape.
        let v = serde_json::to_value(&hash)?;
        let checksum = Checksum {
            algorithm: v["type"].as_str().unwrap_or_default().to_string(),
            value: v["value"].as_str().unwrap_or_default().to_string(),
        };
        Ok(FileEntry {
            path: m.path.clone(),
            key,
            size: m.size,
            checksum,
            mtime_local: Some(local_time(
                SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(u64::try_from(m.mtime_unix).unwrap_or(0)),
            )),
            version_id: uri.version,
        })
    }

    /// A member still matches its snapshot. A missing file or a changed size
    /// or mtime is a different run (refused); any other I/O error — a share
    /// that dropped off the network — is transient and retried next pass.
    fn unchanged(source: &Path, m: &SnapMember) -> Result<(), Error> {
        let meta = match std::fs::metadata(source) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Refused(format!(
                    "member gone after boundary: {}",
                    m.path
                )));
            }
            Err(e) => return Err(e.into()),
        };
        if meta.len() != m.size || unix(meta.modified()?) != m.mtime_unix {
            return Err(Error::Refused(format!(
                "member changed after boundary: {}",
                m.path
            )));
        }
        Ok(())
    }

    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<T, Error> {
        let stream = self
            .remote
            .get_object_stream(self.host.as_ref(), &s3(bucket, key))
            .await?;
        let bytes = stream
            .body
            .collect()
            .await
            .map_err(|e| Error::Io(std::io::Error::other(e)))?
            .into_bytes();
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// When `<control_dir>/<folder name>.complete` was written, if it exists.
fn completion_request(instrument: &Instrument, folder: &Path) -> Option<SystemTime> {
    let dir = instrument.boundary.control_dir.as_ref()?;
    let name = folder.file_name()?.to_string_lossy();
    std::fs::metadata(dir.join(format!("{name}.complete")))
        .and_then(|m| m.modified())
        .ok()
}

fn instrument_json(i: &Instrument) -> serde_json::Value {
    let mut v = json!({ "id": i.id });
    for (k, val) in [
        ("vendor", &i.vendor),
        ("model", &i.model),
        ("software", &i.software),
        ("software_version", &i.software_version),
    ] {
        if let Some(val) = val {
            v[k] = val.clone().into();
        }
    }
    v
}

/// The journal's key for a package: the same name in two buckets is two packages.
fn bucket_package_key(bucket: &str, package_name: &str) -> String {
    format!("{bucket}:{package_name}")
}

fn s3(bucket: &str, key: &str) -> S3Uri {
    S3Uri {
        bucket: bucket.to_string(),
        key: key.to_string(),
        version: None,
    }
}

fn join_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn snap_member(m: &watch::Member) -> SnapMember {
    SnapMember {
        path: slash_path(&m.path),
        size: m.size,
        mtime_unix: unix(m.mtime),
    }
}

fn slash_path(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn unix(t: SystemTime) -> i64 {
    DateTime::<Utc>::from(t).timestamp()
}

/// The instrument's wall clock as the share reports it: no offset, never
/// corrected.
fn local_time(t: SystemTime) -> String {
    DateTime::<chrono::Local>::from(t)
        .naive_local()
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

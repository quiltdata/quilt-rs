# quilt-agent

Unattended instrument-to-cloud mover. It watches instrument run folders, decides
when a run is complete, lands its files in S3, writes a sentinel recording what
was observed, and publishes the run as a fresh Quilt package revision.

- **Outbound HTTPS only.** Reads the instrument's share; never writes to it.
- **Completion is an observation.** One boundary method per instrument:
  `marker_file`; `explicit` (a scheduler or person writes
  `<run folder>.complete` into the agent's `control_dir`); or `size_stable`,
  marked `guessed: true`.
- **Survives kill and outage.** Every step is journalled under `spool_root`; the
  next start resumes where the journal ends and lands the run exactly once.
- **Never moves someone else's `latest`.** Publishing uses
  `quilt_rs::flow::push_revision`, which advances `latest` only when nobody else
  has moved it. The crate's `clippy.toml` forbids pull, reset, certify and
  `push_package`.

```sh
quilt-agent check profile.yaml
quilt-agent status profile.yaml   # one JSON line per run; parked runs say why
QUILT_AGENT_API_KEY=qk_... quilt-agent run profile.yaml
quilt-agent run profile.yaml --bucket-only   # ambient AWS credentials, no registry
```

The profile schema is `schemas/agent-config.schema.json`; the sentinel schema is
`schemas/sentinel.schema.json`. Service wrappers are in `service/`.

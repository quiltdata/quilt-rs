# Quilt Architecture Specification

> **Prerequisite reading**: This document assumes familiarity with Quilt's
> [Mental Model](https://docs.quilt.bio/mentalmodel) — packages, manifests,
> logical/physical keys, registries, and the bucket-as-branch workflow are
> introduced there and used here without re-definition.
>
> **Audience**: Contributors and technical stakeholders who need the design
> commitments, invariants, and reasoning that cannot be recovered from the
> code. Step-by-step mechanics are deliberately absent: the code is the
> authority on what happens, and the rustdoc on the `flow` module documents
> each operation. What lives here is the *why* and the behavior that
> surprises.
>
> **See also**: [`docs/mental-model.md`](mental-model.md) for the four-hash
> state model (`commit`, `remote.hash`, `base_hash`, `latest_hash`) and the
> `UpstreamState` classifier.

## Overview

Quilt is a data package management system that provides Git-like version
control semantics for data files. Packages can be extremely large
(thousands of files, terabytes of data), so the system is designed for
partial downloads and incremental modifications. Files are identified by
cryptographic hash (content-addressed storage): objects are immutable,
identical content is stored once, integrity is verifiable, and content can
be shared across storage locations. Because every file and manifest is
identified by its hash, subtle pipeline differences (byte ordering, JSON
canonicalization, line endings) produce a different hash and break
compatibility.

## The Local Repository (.quilt)

A Domain is the top-level envelope for the entire system: a set of
namespaces, packages, and lineage rooted at a single directory, represented
in code by `LocalDomain`. Its on-disk layout:

```text
.quilt/
├── packages/           # Immutable cache of remote manifests, by bucket
├── installed/          # Local package installations, by namespace
├── objects/            # Content-addressed object store (deduplicated)
└── data.json           # Installation, modification, and commit tracking
```

The load-bearing choice is that a checked-out working file is a **mutable
copy out of `objects/`**, never the canonical object itself. Editing a
working file cannot corrupt the store, and modification detection is a
cheap timestamp-plus-hash comparison against the recorded `PathState`
rather than a re-hash of the store.

The consequence accepted with it: `objects/` and the `packages/` cache are
**never pruned** — not on uninstall, not on reset, not on pull. Content may
be shared across packages and there is no reference counting, so the safe
choice is to leak bytes rather than risk deleting content another package
still addresses.

## Workspace Crate Layout

The workspace splits along an I/O boundary:

- **WASM-safe leaf crates** (`quilt-uri` today): no I/O, compile to
  `wasm32-unknown-unknown`. We expect a few more such extractions —
  likely candidates are checksum / hashing helpers, manifest types, and
  workflow validation (a pure `quilt-workflow`: the config model plus
  the rules-checking gate, no I/O — with reuse value for live
  client-side validation in the UI) — but the bar is "clean API and
  reuse value", not a default path for every portable subset.
- **`quilt-rs`**: native-only library; depends on the leaf crates plus
  `aws-sdk-s3`, `tempfile`, `ignore`, `tokio`.
- **Native consumers**: `quilt-cli`, `quilt-sync/src-tauri`.
- **WASM consumer**: `quilt-sync/ui` depends on the leaf crates only.

### Why separate crates, not feature flags

The serious alternative is one crate with target-gated deps and
`#[cfg]`-stripped I/O modules — `aws-sdk-s3` and friends listed under
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`. That
compiles on both targets and sidesteps Cargo's feature unification.
Extraction is preferred when a subset has a coherent external API;
small or tightly-coupled subsets stay as `#[cfg]`-gated modules in
`quilt-rs`.

A third path sits between these: a subset that is extraction-worthy but
not yet needed as a crate is first consolidated as a self-contained
module inside `quilt-rs` — its own error type, no dependency on the
`Remote` trait — so the eventual crate lift is mechanical rather than a
redesign. `object_hash` and `workflow` are staged this way today;
`workflow` is already pure and I/O-free (its `jsonschema` / `regex` /
`serde_yaml` dependency set builds for `wasm32`), with extraction waiting
on a concrete second consumer.

The extraction case rests on:

- **Named, discoverable surface.** `quilt-uri` has its own `cargo
  doc`, README, and changelog. With cfg-stripping, the WASM-available
  subset is implicit — a UI author has to grep `#[cfg]` attributes
  across `quilt-rs` to know what compiles where.
- **Write-time discipline.** Crate boundaries make portability
  structural: you cannot import `tokio::fs` into `quilt-uri` because
  it is not in `Cargo.toml`. With cfg-stripping you can import it
  into a portable module and only learn at WASM build time.
- **Independent versioning and reuse.** UI does not churn on
  `quilt-rs` releases that do not touch URI logic; external Rust
  consumers can depend on `quilt-uri` without the rest.

Feature flags remain right for variant *implementations* of one fixed
API (TLS backend, allocator). Use crates when *which* API exists
varies by target; use flags when *how* it is implemented varies by
build.

> Release mechanics (which crates are published, where, and how) live
> in [docs/releases.md](releases.md).

## Manifest Semantics

Manifests are stored as JSONL and content-addressed by their top-level
hash (`top_hash`). Two properties worth knowing beyond the format itself:

- A row's `physical_key` is a URI treated as **read-only**: on S3, object
  versioning makes accidental overwrites recoverable; on local
  filesystems, immutability is only a convention enforced by
  content-addressed naming.
- Package-level and per-row user metadata are part of the hashed content —
  a metadata change is a new revision.

A **workflow** is a package-level configuration stored as a YAML file in
the remote registry and referenced from the package's manifest header. It
defines how the package must be validated on push (required metadata
fields, file constraints) and supplies default property values when the
user does not specify them. The push and recommit flows resolve the active
workflow from the remote and apply it to the manifest before upload.

## Operation Contracts

Operations are the `flow::` free functions re-exported from
`quilt-rs/src/flow.rs`; the short names (`flow::commit`, `flow::push`, …)
are aliases for longer underlying functions (`commit_package`,
`push_package`, `create_status`). The mechanics live in those modules;
this section records only the contracts and the behavior that is easy to
get wrong.

### Create

A newly created package records an initial commit covering the source
directory (like `git init` plus a first commit), so its status is clean.

### Status and `.quiltignore`

`.quiltignore` uses `.gitignore` syntax but differs in effect on tracked
files: ignoring an already-tracked file untracks it, so it surfaces as
`Removed` and disappears from the next committed revision. This is
intentional — `.quiltignore` controls which files belong in the package —
but it means adding a pattern can delete a file from the next published
revision, where `.gitignore` would have left it tracked.

### Set Remote (recommit)

Which checksum algorithm an upload is written with is not a client-side
choice: the remote host's config declares the algorithm it expects, and
the client conforms. A local-only package was hashed under the local
default, so binding it to a remote re-hashes the existing commit
("recommit") before the first push; without this the push would fail with
a hash mismatch. The remote binding is persisted even when the recommit
itself fails (e.g. not logged in yet), so Set Remote still succeeds —
but push does not redo the recommit; it fails with that top-hash
mismatch until Set Remote is re-run (idempotent while the package has
never been pushed).

### Commit

Package-level metadata is durable across commits: the caller passes an
explicit intent (`UserMeta`) — `Keep` inherits the previous revision's
metadata, `Clear` drops it, `Set` replaces it — and callers with no
opinion pass `Keep`, so a background commit can never strip metadata by
omission.

### Push

- Pushing with no pending commit is a **no-op success**
  (`certified_latest = true`), not an error.
- Rows identical on the remote are not re-uploaded; only changed content
  moves.
- The remote `latest` tag is re-read mid-push; the certify decision uses
  that fresh value, not the possibly-stale one from the last status
  check.
- The push certifies its revision as `latest` when any of: it is the
  first push, the caller was tracking the tip
  (`base_hash == latest_hash`), or no latest tag exists yet.
- **First push always certifies.** `is_first_push` is captured before
  `base_hash` is filled in, and it short-circuits the tracking check: the
  very first push of a package certifies its revision as `latest`
  unconditionally — even when the remote already carries a different
  `latest` (the state the classifier reports as `Diverged` when a
  teammate published the same namespace first). The rationale recorded in
  the code: the user explicitly pushed this version. Only *subsequent*
  pushes respect a moved `latest`.
- `certified_latest = false` means the upload itself succeeded but
  someone else pushed in the meantime, so the `latest` tag was not
  moved.

### Publish

`flow::publish` (commit-then-push) is the authoritative composition;
desktop and CLI share it instead of re-sequencing commit + push at call
sites. Its failure semantics make it retryable: a commit failure aborts
before any upload is attempted, and a push failure leaves the local
commit in place, so re-running publish resumes as a push-only call.

### Certify Latest (Promote)

The user-invoked composition of push + certify, run from the merge page:
it forces certification even when push declined to (the remote `latest`
has moved since the user's base). It is intentionally last-writer-wins on
the tag — if another client moves `latest` between the inner push and the
outer tag write, that move is silently overwritten. This is the semantic
the merge page asks for, but it makes a non-certifying push and the
merge-page action asymmetric: push respects a concurrent `latest`;
promote does not.

### Pull

Surgical reconcile, not fast-forward: pull updates only the remote-changed
tracked paths the user did not touch and keeps non-conflicting local work in
place. It still refuses on a pending commit and on divergence, but no longer
on any working-tree change — that gate is gone. A dry-run `classify_pull`
verdict (`PullOutcome`) drives the decision: `CleanUpdate` (no local changes),
`KeepsLocalChanges` (non-conflicting local work survives), or `Blocked` (a path
changed on both sides with a different result — the whole pull aborts atomically
with the conflicting paths named). Immediately before applying, every touched
path is re-hashed against its base row (verify-before-mutate); any drift — a
raced edit or delete — aborts the pull as a retryable conflict with zero
mutation. `snapshot_for_pull` resolves `latest` once (refreshing `latest_hash`
and naming the fetch), short-circuits when already up to date, caches the latest
manifest, and takes the status walk last, so no network happens between
classification and mutation. Paths absent from the new revision are removed from
the working tree and from tracking — logged, not an error.

### Refresh Latest

`flow::refresh_latest_hash` is the only path that mutates `latest_hash`
without an explicit user action; it touches nothing but the lineage.
There is no freshness model — the classifier trusts the believed tip as
of the last refresh, which is why autosync refreshes at the top of every
tick. Autosync's autopull tick is a state-driven dispatcher over the same
operations the manual UI buttons invoke: refresh, classify, then route to
pull (`Behind` with a non-conflicting `PullOutcome` — local work is preserved)
or publish (changes or a pending commit, quiet tree). A `Behind` tree with a
`Blocked` outcome pauses with `PausedReason::PullConflict`; `Diverged` pauses
the namespace. Resolution of either is user-action only.

### Uninstall

Removes tracking, installed manifests, and the working directory — but
deliberately leaves `objects/` and the `packages/` cache (shared content,
no reference counting).

## Resolving Diverged

The `UpstreamState` classifier (see
[`docs/mental-model.md`](mental-model.md)) reports `Diverged` when both
sides moved past the merge base. A `Diverged` package leaves that state
via one of two remediation flows: Certify Latest biases local-wins by
pushing the local commit (if any) and tagging it as `latest`; Reset Local
biases remote-wins by discarding the local commit chain and re-installing
the remote `latest`. Pull is the non-conflict path from `Behind` — it
reconciles remote changes surgically and refuses on divergence.

These remediation flows intentionally do **not** implement merge in the
Git/Mercurial/SVN sense. Users familiar with those tools should expect
the following gaps:

- **No merge operation, no merge commit.** Resolving `Diverged` is a
  binary, package-level choice — Promote (local wins) or Overwrite
  (remote wins) — that produces no node combining the two sides.
  After Promote, the previously-certified remote hash is unreachable
  from any lineage entry; there is no graph node linking the two
  sides of a `Diverged` resolution.
- **No per-file granularity.** "Pick file A from mine, file B from
  theirs" is not expressible. The manifest is the unit of choice,
  not the file.
- **No integrative pull from Diverged.** Pull reconciles only from
  `Behind` and refuses on divergence.
  From `Diverged`, the only "remote-wins" path is Reset Local, which
  discards the local commit chain — there is no equivalent of
  `git pull --rebase` that would replay local commits on top of
  remote.
- **Promote silently overwrites the remote tag.** A concurrent
  teammate certification is overwritten last-writer-wins; there is no
  `--force` opt-in gesture.
- **Reset has no reflog.** Git's `reset --hard` is reflog-recoverable
  for ~90 days; Reset Local has no such safety net. The discarded
  local commit's installed manifest may linger on disk under
  `.quilt/installed/` but is unreachable from any lineage state —
  effectively orphaned garbage.

The design rationale is that data packages are predominantly binary
or large (Parquet, FASTQ, HDF5), where line-level three-way merge is
meaningless. Forcing a whole-manifest choice avoids
"stuck-in-the-middle" partial-merge state and keeps the resolution
model simple, at the cost of any granularity finer than "your
manifest or theirs."

## Hash Algorithms

Three checksum algorithms are supported, reflecting the historical
evolution of the format:

1. **SHA256** — the original algorithm: a single digest over the entire
   file.
2. **SHA256-Chunked** — added to speed up large-file uploads. The file is
   split into fixed-size chunks, each chunk is hashed, and the chunk
   hashes are combined into a top digest (aligning with S3
   multipart-upload boundaries so chunks can be hashed in parallel with
   the upload).
3. **CRC64** — the current default. AWS computes CRC64 server-side on
   every object, so we can trust S3's checksum rather than re-hashing the
   file client-side.

Each new algorithm became the default for freshly created packages, but
all three remain fully supported for reading existing packages. The
`ObjectHash` enum provides a unified interface across them. Which
algorithm a new upload is *written* with is the remote's decision, not
the client's — see Set Remote above.

## Network Resilience

Remote I/O goes through a shared layer rather than being inlined per call
site:

- **Shared HTTP client**: a single reqwest client with connect and
  per-request timeouts, plus exponential-backoff retries for transient
  failures. Non-2xx responses are logged with status, URL, and a
  truncated body so failures remain diagnosable.
- **Fresh S3 credentials**: every signed request fetches current
  credentials from the Quilt auth backend instead of a cached snapshot,
  avoiding `ExpiredToken` errors in long sessions.
- **Single-flight refresh per host**: when many requests hit expired
  credentials at once, they coalesce onto one refresh per host rather
  than stampeding the auth backend.

## Storage and Security Conventions

- S3 stores are versioned — each revision of an object gets a
  `versionId`. Local stores are flat and simply overwrite objects; when
  using a flat store, the registry caches each known version to avoid
  overwrites.
- Objects in the local store are immutable **by convention**
  (content-addressed naming discourages overwriting, but the filesystem
  does not enforce it).
- Manifests contain only file metadata (paths, hashes, sizes) — never
  credentials. Authentication is handled externally (AWS credentials,
  OAuth).

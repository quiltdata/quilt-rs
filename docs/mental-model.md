<!-- markdownlint-disable MD013 -->
# State Mental Model

This document describes the state model of an installed quilt-rs
package: what fields encode that state, how the `UpstreamState`
classifier derives a verdict from them, and which operations move
which fields. Phase mechanics, on-disk layout, hash algorithms, and
network behavior live in [`docs/architecture.md`](architecture.md).

The content here changes far less often than architecture.md: the
four hashes have not been renamed since the `Option<ManifestUri>`
refactor (#594) and the classifier truth table has been stable since
#677.

## The four hashes

All four name the same kind of thing — a top-hash of a manifest —
but they answer different questions about *when in time* and *where
on disk* that manifest lives.

| Field | Type | Question it answers |
| --- | --- | --- |
| `commit.hash` | `Option<CommitState>.hash: String` | Is there an unpushed local revision, and what is its top hash? |
| `remote.hash` | `Option<ManifestUri>.hash: String` | Which remote revision did we install / last push to? |
| `base_hash` | `String` | Which remote revision is our local state built on top of? |
| `latest_hash` | `String` | What did the remote `latest` tag point at, last time we checked? |

A useful framing:

- `commit.hash` is the **local frontier** — never written to the
  remote until push consumes it.
- `remote.hash` is the **remote address** — the specific manifest
  the local copy ultimately came from.
- `base_hash` is the **merge base** — what we agreed with the remote
  we are diverging from.
- `latest_hash` is **what we think the remote tip is** — informational,
  may be stale.

### Why `base_hash` and `remote.hash` are separate

Installing a non-latest revision (`pkg@<old_hash>`) leaves
`base_hash = remote.hash = <old_hash>` but `latest_hash` is whatever
`resolve_tag("latest")` returned. That state is `Behind`, and three
fields cannot represent it.

## Classifier truth table

The `UpstreamState::from(&PackageLineage)` impl
(`quilt-rs/src/lineage/package.rs:102`) is a short cascade:

```text
remote_uri = None                     → Local
remote.bucket = ""                    → Local         (defensive, hand-edited)
remote.hash = "" ∧ latest_hash = ""   → Local         (genuine first push)
remote.hash = "" ∧ latest_hash ≠ ""   → Diverged      (teammate already published)
otherwise:
  ahead  = (base_hash ≠ current_hash())
  behind = (base_hash ≠ latest_hash)
  (false, false) → UpToDate
  (false, true ) → Behind
  (true,  false) → Ahead
  (true,  true ) → Diverged
```

with `current_hash() = commit.hash ?? remote.hash ?? base_hash`
(empty `base_hash` reads as `None`, not `""`; same file, `:173`).

Plain-English read:

- **ahead** = "the locally-current revision is no longer the one I
  started from". True exactly when there is an unpushed `commit`.
  After a successful push, `commit = None` and `remote.hash = base_hash`,
  so `current_hash() = base_hash` and `ahead` is false again.
- **behind** = "the remote `latest` tag points somewhere other than
  my base". True when someone else pushed a newer revision under the
  same namespace.

The `UpstreamState` enum has a sixth variant, `Error`, that the
classifier's `From` impl does not produce. It is surfaced by the
status computation when computing the state itself fails — see
`InstalledPackageStatus::error()`. The truth table covers only the
five "successful classification" outcomes.

## Lifecycle: who writes each field, when

Cross-reference [`docs/architecture.md`](architecture.md) for the
phase definitions; this table only names *which fields each phase
mutates*.

| Phase | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` |
| --- | --- | --- | --- | --- |
| `flow::install_package` | — | install-time hash | install-time hash | `resolve_tag("latest")` |
| `flow::create` (local-only) | initial top hash | — | `""` | `""` |
| `InstalledPackage::set_remote` | unchanged | `""` (empty until first push) | unchanged | unchanged |
| `flow::commit` | new top hash | unchanged | unchanged | unchanged |
| `flow::push` | `None` (taken) | new uploaded hash | first push only → new hash | only if push certified |
| `flow::certify_latest` | — (cleared by inner push) | already set | ← `latest_hash` (via `update_latest`) | ← new manifest hash |
| `flow::pull` (fast-forward) | must be `None` | ← `latest_hash` | ← `latest_hash` | already advanced |
| `flow::reset_to_latest` | **`None`** (cleared since #677) | ← `latest.hash` | ← `latest.hash` | ← `latest.hash` |
| autopull tick (`flow::refresh_latest_hash`) | — | — | — | ← `resolve_tag("latest")` |

Two non-obvious points:

1. **Only `latest_hash` moves passively.** A background autopull tick
   can update it without any user action. Everything else moves only
   when the user (or autosync) runs a flow.
2. **`commit.hash` is cleared on push by `.take()`**
   (`flow/push.rs:105`). That is why `current_hash()` falls back
   through `commit → remote.hash → base_hash`: after a push the local
   frontier is `remote.hash`; before any push at all on a freshly-
   `create`d local-only package, it is `commit.hash`.

## A short walkthrough

Bucket `b`, namespace `f/a`, remote has revision `H1` tagged `latest`.

| Action | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` | State |
| --- | --- | --- | --- | --- | --- |
| install `H1` | — | `H1` | `H1` | `H1` | UpToDate |
| edit + commit → `H2` | `H2` | `H1` | `H1` | `H1` | Ahead |
| teammate pushes `H3`; autopull tick | `H2` | `H1` | `H1` | `H3` | Diverged |
| Promote (Certify Latest) | — (cleared) | `H2` | `H2` | `H2` | UpToDate |

Or, from the `Diverged` row, the other resolution:

| Action | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` | State |
| --- | --- | --- | --- | --- | --- |
| Overwrite (Reset Local) | — | `H3` | `H3` | `H3` | UpToDate |

And the first-push asymmetry that motivated #677:

| Situation | `remote.hash` | `latest_hash` | State |
| --- | --- | --- | --- |
| `set_remote` to an empty namespace, no push yet | `""` | `""` | Local |
| `set_remote` to a namespace someone else owns | `""` | `<their hash>` | Diverged |

Before #677 both first-push rows collapsed to `Local`, which is why
a teammate could silently lose work on the very first push.

## Writer invariants

A handful of rules that every code path that mutates lineage must
preserve. Stated as invariants, not as enforced types — there is no
compile-time guard for most of these (the hashes are `String`s).

- `commit.hash` is the **only** hash that can be non-empty while no
  remote revision exists (e.g. after `flow::create` of a local-only
  package).
- `remote.hash` and `base_hash` both empty ⇔ first push has not been
  made.
- `set_remote` rejects empty buckets at the write boundary; the
  classifier still defends against the hand-edited case anyway
  (`lineage/package.rs:114`).
- On a successful push, `commit` becomes `None`. On *first* push
  specifically, `base_hash` is pinned to the uploaded hash; without
  that pin the package would classify as `Diverged` immediately
  after push.
- `update_latest` sets `base_hash` and `latest_hash` together, never
  one without the other.
- `flow::reset_to_latest` clears `lineage.commit` (since #677). A
  stale commit after reset would let a subsequent `certify_latest`
  resurrect the discarded revision — its installed manifest is still
  on disk.
- `latest_hash` has no freshness model: it is whatever
  `resolve_tag("latest")` returned the last time *anything* asked. On
  a long-quiet client the verdict the classifier produces from it is
  stale and there is currently no way to express "I don't know what
  latest is right now."

## Pointers

- Classifier: `quilt-rs/src/lineage/package.rs:102`
- `current_hash()`: `quilt-rs/src/lineage/package.rs:173`
- `update_latest()`: `quilt-rs/src/lineage/package.rs:185`
- Field-write sites: every file in `quilt-rs/src/flow/`; the
  lifecycle table above maps each phase to the fields it mutates.
- Background refresh: `flow::refresh_latest_hash`, called by
  autosync's autopull tick.

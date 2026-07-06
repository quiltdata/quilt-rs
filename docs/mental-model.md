# State Mental Model

This document describes the state model of an installed quilt-rs
package: what fields encode that state, how the `UpstreamState`
classifier derives a verdict from them, and which operations move
which fields. Design commitments, operation contracts, hash
algorithms, and network behavior live in
[`docs/architecture.md`](architecture.md); the step-by-step mechanics
live in the code.

## The four hashes

All four name the same kind of thing — a top-hash of a manifest —
but they answer different questions about *when in time* and *where
on disk* that manifest lives.

| Field | Type |
| --- | --- |
| `commit.hash` | `Option<CommitState>.hash: String` |
| `remote.hash` | `Option<ManifestUri>.hash: String` |
| `base_hash` | `String` |
| `latest_hash` | `String` |

- `commit.hash` is the **local frontier** — never written to the
  remote until push consumes it.
- `remote.hash` is the **remote address** — the specific manifest
  the local copy ultimately came from.
- `base_hash` is the **merge base** — what we agreed with the remote
  we are diverging from.
- `latest_hash` is **what we think the remote tip is** — informational,
  may be stale.

All four exist because installing a non-latest revision needs an
install-time `base_hash` distinct from a moving `latest_hash` to
express `Behind`.

## Classifier truth table

The `UpstreamState::from(PackageLineage)` impl is a short cascade:

```text
remote_uri = None                     → Local
remote.bucket = ""                    → Local         (defensive, hand-edited)
remote.hash = "" ∧ latest_hash = ""   → Local         (genuine first push)
remote.hash = "" ∧ latest_hash ≠ ""   → Diverged      (teammate already published)
otherwise:
  ahead  = (base_hash ≠ current_hash())
  behind = (base_hash ≠ latest_hash)
  (ahead, behind):
  (false, false) → UpToDate
  (false, true ) → Behind
  (true,  false) → Ahead
  (true,  true ) → Diverged
```

with `current_hash() = commit.hash ?? remote.hash ?? base_hash`
(empty `base_hash` reads as `None`, not `""`).

The enum has a sixth variant, `Error`, that the `From` impl does not
produce — it is surfaced by `InstalledPackageStatus::error()` when
status computation itself fails.

## Lifecycle: who writes each field, when

Cross-reference [`docs/architecture.md`](architecture.md) for the
operation contracts; this table only names *which fields each
operation mutates*.

| Operation | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` |
| --- | --- | --- | --- | --- |
| `flow::install_package` | — | install-time hash | install-time hash | `resolve_tag("latest")` |
| `flow::create` (local-only) | initial top hash | — | `""` | `""` |
| `InstalledPackage::set_remote` | unchanged | `""` (empty until first push) | unchanged | unchanged |
| `flow::commit` | new top hash | unchanged | unchanged | unchanged |
| `flow::push` | `None` (taken) | new uploaded hash | first push → new hash; certifying push → via `update_latest` | re-resolved from remote mid-push; ← new hash if certified |
| `flow::certify_latest` | — (cleared by inner push) | already set | ← `latest_hash` (via `update_latest`) | ← new manifest hash |
| `flow::pull` (fast-forward) | must be `None` | ← `latest_hash` | ← `latest_hash` | already advanced |
| `flow::reset_to_latest` | **`None`** (cleared since #677) | ← `latest.hash` | ← `latest.hash` | ← `latest.hash` |
| `flow::refresh_latest_hash` | — | — | — | ← `resolve_tag("latest")` |

Only `latest_hash` moves passively: every status check refreshes it
via `flow::refresh_latest_hash`. Autosync's autopull tick is a
state-driven dispatcher on top of that primitive — it refreshes,
classifies, then routes to `flow::pull` or `flow::publish` when the
state allows; both call the same code paths the manual UI buttons
invoke. `commit.hash` is cleared on push by `.take()`, which is why
`current_hash()` falls back through `commit → remote.hash →
base_hash`.

## Walkthrough

Bucket `b`, namespace `f/a`, remote has revision `H1` tagged `latest`.

| Action | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` | State |
| --- | --- | --- | --- | --- | --- |
| install `H1` | — | `H1` | `H1` | `H1` | UpToDate |
| edit + commit → `H2` | `H2` | `H1` | `H1` | `H1` | Ahead |
| teammate pushes `H3`; autopull tick | `H2` | `H1` | `H1` | `H3` | Diverged |
| Promote (Certify Latest) | — | `H2` | `H2` | `H2` | UpToDate |
| Overwrite (Reset Local) — alternate exit from Diverged | — | `H3` | `H3` | `H3` | UpToDate |

### First-push race

Local-only package created as `H1`; a teammate has already pushed
`H2` (tagged `latest`) to the same namespace.

| Action | `commit.hash` | `remote.hash` | `base_hash` | `latest_hash` | State |
| --- | --- | --- | --- | --- | --- |
| create → `H1` | `H1` | — | `""` | `""` | Local |
| `set_remote` (recommit → `H1'`) | `H1'` | `""` | `""` | `""` | Local |
| status refresh sees teammate's `H2` | `H1'` | `""` | `""` | `H2` | Diverged |
| push | — | `H1'` | `H1'` | `H1'` | UpToDate |

Push exits this `Diverged` unaided: an empty `base_hash` on entry
marks the push as first, which short-circuits the certify guard —
the first push always certifies, de-certifying the teammate's `H2`
without a merge-page gesture. Only non-first pushes decline the
tag and report `certified_latest: false` (see the Push contract in
[`docs/architecture.md`](architecture.md)).

## Writer invariants

Hashes are `String`; these are conventions, not type-enforced rules.

- `commit.hash` is the **only** hash that can be non-empty while no
  remote revision exists (e.g. after `flow::create` of a local-only
  package).
- `remote.hash` and `base_hash` both empty ⇔ first push has not been
  made.
- `set_remote` rejects empty buckets at the write boundary; the
  classifier still defends against the hand-edited case anyway.
- `flow::reset_to_latest` clears `lineage.commit` (since #677). A
  stale commit would let a subsequent `certify_latest` resurrect the
  discarded revision — its installed manifest is still on disk.
- `latest_hash` has no freshness model: it is whatever
  `resolve_tag("latest")` returned the last time *anything* asked.

See `quilt-rs/src/lineage/package.rs` for the classifier and
`current_hash()`; field-write sites are the `flow::*` functions
referenced in the lifecycle table.

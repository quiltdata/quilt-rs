# Code Review — `sync-shallow-pull` (surgical pull)

- **Date:** 2026-07-22
- **Scope:** `git diff main...sync-shallow-pull` — 23 files, +1281/−202 (commits `f7d9c5bf`…`99a2972b`)
- **Method:** 8 finder angles (line-by-line, removed-behavior, cross-file, reuse, simplification, efficiency, altitude, conventions), ~40 raw candidates deduped to 14, each verified by an independent pass. 11 CONFIRMED, 1 PLAUSIBLE, 1 REFUTED; 10 most severe reported below.

## Findings (most severe first)

### 1. Add/add conflict classified `KeepsLocalChanges`, not `Blocked` — silent data loss

`quilt-rs/src/flow/pull_outcome.rs:56` · correctness · **CONFIRMED**

`remote_delta` only iterates base-manifest rows (doc: latest-only paths are "out of scope (sparse checkout)"), so a path added both remotely and locally with different content produces no delta entry and `classify_pull` files it under `KeepsLocalChanges` — the UI even says the local changes are safe.

**Failure scenario:** User adds `report.txt` locally while a teammate's pushed revision also adds `report.txt` with different content. Pull advances base to latest without touching the file, status then reports it as *Modified*, and the next autosync publish commits and pushes the local version over the teammate's freshly added file with no conflict ever surfaced.

### 2. Both-removed path leaves a stale lineage entry that permanently wedges the package

`quilt-rs/src/flow/pull.rs:86` · correctness · **CONFIRMED**

The touch-set filter `.filter(|p| !status.changes.contains_key(p))` excludes trivially-resolved both-removed paths, so `uninstall_paths` (the only code that does `lineage.paths.remove`) never runs for them, and `InstalledPackage::pull` persists a lineage that tracks a path the new base manifest has no row for.

**Failure scenario:** User deletes tracked `a.txt` locally and the remote latest also removed it. `classify_pull` calls this trivially resolved, the pull proceeds and succeeds, and every subsequent `create_status` hits `None => Err(UriError::ManifestPath("path … not found in installed manifest"))` — status, pull, and commit all fail, and every autosync tick errors, until `lineage.json` is hand-edited or `reset_to_latest` is run.

### 3. Mid-pull failure after uninstall yields a false `PullConflict`

`quilt-rs/src/flow/apply_update.rs:49` · correctness · **CONFIRMED**

`apply_latest_update` deletes touched working-tree files (`uninstall_paths` → `storage.remove_file`) *before* the network-dependent `install_paths`, and lineage is persisted only on success.

**Failure scenario:** Clean tree, remote modified `data.csv`. Uninstall deletes the file, then `install_paths` fails on a network drop. On disk the file is gone but still tracked: next status reports local-*Removed* vs remote-*Modified*, `classify_pull` returns `Blocked` (pinned by `local_remove_vs_remote_modify_blocks`), and retry hits `PullConflict` — autosync pauses telling the user to commit changes that don't exist, contradicting `apply_update.rs`'s "callers treat `pull`/`reset` as retryable" doc comment. (The old pull's destructive window was larger, but it failed into a plain refusal, not a misleading conflict.)

### 4. Edit made during pull's fetch window is silently overwritten

`quilt-rs/src/flow/pull.rs:67` · correctness · **PLAUSIBLE**

Pull classifies against a caller-supplied status snapshot, then deletes/reinstalls touched files. A file edited after the status walk but before apply is absent from `status.changes`, lands in the touch-set if remote-changed, and is uninstalled+reinstalled — the fresh edit deleted with no conflict signal.

**Failure scenario:** The TOCTOU pre-existed, but on `main` any pending change made pull refuse, so dirty trees never reached the race; now `KeepsLocalChanges` pulls proceed and autosync triggers them automatically, exposing strictly more scenarios.

### 5. One failed dry-run leaves Pull disabled indefinitely

`quilt-sync/ui/src/pages/installed_package/content.rs:176` · correctness · **CONFIRMED**

`package_pull_outcome` errors are swallowed with `.ok()`, and the detail-page `LocalResource` closure captures a plain `String` (tracks no signals), so it runs exactly once per instantiation with no retry or error surface.

**Failure scenario:** A behind package's one-shot dry-run invoke fails (network blip, expired session). `None` keeps `pull_disabled` true and the banner stuck on "Checking for updates…" until the page re-instantiates. The list page (`installed_packages_list.rs:361`) has the same swallow, re-running only when the status signal changes — which doesn't happen while the package stays behind with autosync off or paused. On `main` the button was synchronous (`disabled=has_changes`) with no network dependency.

### 6. Autosync-detected conflict shows a paused row with the wrong guidance

`quilt-sync/src-tauri/src/autopull/tick.rs:186` · correctness · **CONFIRMED**

`PullOutcome::Blocked` pauses the row as `"paused"`, but every conflict affordance — the two-phase pull-outcome resources, the Pull button and popover, the banner's conflict copy — is gated on `status == "behind"`, and both toast listeners drop any reason `!= "other"`.

**Failure scenario:** The user sees generic `PAUSED_GUIDANCE` ("…then push manually to resume" — the wrong remediation for a pull conflict) plus a bare comma-joined file list with no hint those files are conflicts, while the Blocked messaging the PR built stays unreachable.

### 7. "package is already up-to-date" built at two sites, exact-string-matched by the tick

`quilt-rs/src/flow/pull.rs:71` · reuse · **CONFIRMED**

The literal is constructed at the base-hash guard (line 57) and the new `PullOutcome::UpToDate` classify arm (line 71); `tick.rs:71` compares `msg == "package is already up-to-date"` to treat the classify-vs-apply race as benign.

**Failure scenario:** Tests pin only the guard site's message, and the tick test constructs the string independently — so rewording the classify arm compiles, passes CI, and turns every up-to-date race into a permanent `PausedReason::Other` pause. The typed alternative is proven in the same match arm (`PackageOpError::PullConflict` is matched structurally).

### 8. Per-tick dry-run before pull is redundant for routing

`quilt-sync/src-tauri/src/autopull/tick.rs:181` · efficiency · **CONFIRMED**

Every arm of the `match outcome` is reachable via `package_pull`'s own error path: `Blocked` maps to the same `PullConflict` pause `classify_sync_err` produces, `UpToDate` maps to `Ok` via the string match, and `CleanUpdate`/`KeepsLocalChanges` route identically into `package_pull`.

**Failure scenario:** Per Behind tick the code performs 3 working-tree status walks and 5 latest-tag resolutions where 1 of each is strictly needed, and `classify_pull` runs twice with a code-documented race window ("tip moved … between status and classify"). `flow::pull` already computes and logs the `PullOutcome` but returns only a `ManifestUri` — returning it (or routing on the error alone) removes the duplicate work.

### 9. `remote_delta` computed twice; touch-set re-derives the classifier's partition

`quilt-rs/src/flow/pull.rs:83` · simplification · **CONFIRMED**

`classify_pull` builds the base→latest delta and discards it; `pull.rs:83` recomputes it and applies a blanket "skip anything the user touched" filter that is only correct because `classify_pull` already `Blocked` non-agreeing both-changed paths.

**Failure scenario:** No type or test ties the two derivations together, so a change to `same_resulting_content` semantics silently desynchronizes them — findings 1 and 2 are exactly such divergences. Having `classify_pull` return the delta or per-path disposition fixes the coupling, the double O(rows) pass, and both correctness bugs in one change.

### 10. Manifest parsed up to 3× per pull; first assignment is dead

`quilt-rs/src/flow/apply_update.rs:57` · efficiency · **CONFIRMED**

`*manifest = cache_remote_manifest(...)` (full parse) is unconditionally overwritten ten lines later by `Manifest::from_path` on a byte-identical copy (`copy_cached_to_installed` is a pure `storage.copy`); in the pull path, `pull.rs:64` had already parsed the same manifest to classify.

**Failure scenario:** Every pull pays up to three full O(rows) manifest deserializations where one suffices — call `cache_remote_manifest` for its caching side effect without assigning, or pass the parsed manifest in from the caller.

## Refuted

- **"Parse-equal but hash-different latest manifest strands a package Behind forever"** (`pull.rs:69`) — the top hash is computed from exactly the parsed fields (`TopHasher` serializes the parsed header/rows), so `Manifest` equality implies hash equality and the guard at `pull.rs:55` already rejects the case. Not constructible from contract-conforming publishers.

## Confirmed but below the cut (low severity)

- `tick.rs:181-184` — the dry-run's blanket `.map_err(WatchError::Transient)` bypasses the `LoginRequired` classification the status call gets; mitigated because the next tick's status call re-encounters the login error, so the login affordance is delayed by one backoff (~2 s), not lost.
- `tick.rs` (pull branch) — a successful pull returns the pre-pull `has_changes`; when all local changes were trivially resolved, the UI counts phantom pending changes for one tick interval.
- `installed_package.rs:514-530` — `pull_outcome` performs 2 tag resolutions, 2 manifest parses, and 4 lineage reads per Behind dry-run and hand-assembles the latest `ManifestUri` instead of using `resolve_tag`'s result; largely subsumed by finding 8.

## Conventions (fix before PR)

- `quilt-rs/CHANGELOG.md` — the v0.34.0-alpha3 entry still has the literal placeholder autolink `…/pull/NNN`; substitute the real PR number before merge.
- `quilt-sync/src-tauri/src/commands/package_ops.rs:316,334` — return types use inline `quilt::flow::PullOutcome`; CLAUDE.md's import-style rule asks for a `use` at the top of the file.
- `quilt-rs/src/flow/pull_outcome.rs:13,160` — fully-qualified `serde::Serialize`/`serde::Deserialize` in the derive and `multihash::Multihash::<256>` in the test helper; crate convention imports both.

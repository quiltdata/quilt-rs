# QuiltSync Telemetry

> **Audience**: Contributors working on QuiltSync who need to add an event, read
> a log, or find out where a failure went. This is the *mechanics*. The design
> commitments — why the sinks split the way they do, why an event names an
> outcome, why release health is off — live in the spec corpus under
> `arch/comp/quilt-sync/node.md`.
>
> **Scope**: `quilt-sync` only. `quilt-rs` and `quilt-cli` emit `tracing` events
> but configure no sinks; the desktop app is what installs the subscriber.

## Three sinks, three questions

| Sink | Answers | Where it goes |
| --- | --- | --- |
| **Analytics** (Mixpanel) | What did people do, and did it work? | Off-machine, per-event |
| **Crash reporter** (Sentry) | What broke that we have to fix? | Off-machine, per-fault |
| **Log file** | What happened on *this* machine, in order? | Local disk, user-visible |

The split is by *question*, not by severity, and that is load-bearing in both
directions. A **recoverable refusal** — a package that does not satisfy a
workflow, a role without access, a cancelled dialog — is an analytics outcome,
not a crash report: the user can resolve it, and an issue list where nothing is
actionable buries the real faults. A **fault** is ours to fix and belongs to the
crash reporter. Counting *runs* is an analytics question too, which is why
session tracking is off in Sentry.

Nothing leaves a local build. `Sinks::resolve()` reads the build profile alone:
a debug build never constructs a Mixpanel client or a Sentry client, so no stray
`.env` credential can make your machine emit. Analytics prints to the terminal
instead (`telemetry(dry-run) <event> {…}`), and faults print rather than upload.

## Logs

**Where.** `<app-data>/logs/quilt-sync.<date>.log`, rotated daily:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/com.quiltdata.quilt-sync/logs/` |
| Linux | `~/.local/share/com.quiltdata.quilt-sync/logs/` |
| Windows | `%LOCALAPPDATA%\com.quiltdata.quilt-sync\logs\` |

**Levels.** Each sink filters for itself, so one choice cannot starve another:

- **File**: `quilt_sync=debug,quilt_rs=debug,quilt_uri=debug,warn`
- **Crash reporter**: `quilt_sync=info,quilt_rs=info,warn`

The trailing `warn` is the part that matters. The dependency tree — an HTTP
stack, the AWS SDK, a filesystem watcher — is far chattier than this app, so a
bare `debug` would bury three hundred of our own statements under transport
noise. The crash reporter sits a level lower than the file on purpose: it leaves
the machine and is billed per event, and `debug` is where paths and package
names appear, so the gap is a privacy control as much as a cost one.

**Retention is ten files, which is not ten days.** `max_log_files(10)` keeps the
ten most recent files, pruning runs only on rotation, and rotation happens only
when a write crosses the date boundary. So the window is *the last ten days on
which the app wrote a line* — unbounded in wall-clock time, with the current
file permanently beyond the pruner's reach and no byte cap anywhere. A quiet
install can hold a file from months ago; a busy one holds ten days. Read as a
calendar window it is simply wrong, and that misreading is the reason this
paragraph exists.

**The tail survives a quit, not a crash.** Lines go to a background writer, so
whatever is still queued when the process ends is lost. `App::run`'s exit callback
in `main.rs` drops the writer's guard, which drains the queue — up to about a second
of the quit. A `kill`, a signal or an aborting panic never reach it.

**Turning up the volume.** `QUILTSYNC_LOG` **replaces** the defaults for both
sinks:

```bash
QUILTSYNC_LOG=quilt_sync=trace,quilt_rs=trace just start
```

It replaces rather than merges, so a narrow override is genuinely narrow —
`QUILTSYNC_LOG=quilt_sync=trace` drops the dependency floor along with
everything else, which is ordinary `RUST_LOG` behaviour. Note this is a
**developer** convenience, not a user-facing escape hatch: an app launched from
the OS shell inherits no terminal environment, so "set a variable and reproduce
it" is not an instruction anybody can follow. The defaults have to be right on
their own.

**Choosing a level for new code.** The cost of a statement is its level times
its **call rate**, and a statement in a timer-driven path has an unbounded rate.
The autosync tick runs every 30 seconds over every package, so anything on that
path wants `trace`, not `debug` — a `debug!` per file per tick once produced a
211 MiB log in seventeen minutes of an *idle* app. Before adding a `debug!`, ask
how often the enclosing function runs; if the answer is "on a timer" or "per
file", it is `trace!`.

Never log a whole collection. `{:?}` on a `Vec<PathBuf>` was 52 KB per call and
88% of that 211 MiB file; log a count plus a bounded sample instead.

## Adding an event

Events are a closed vocabulary in
[`telemetry/event.rs`](../quilt-sync/src-tauri/src/telemetry/event.rs). Adding
one:

1. **Add a variant** to `MixpanelEvent`. The wire name is the snake-case form of
   the variant, produced by serde — do not write it out anywhere.
2. **Pick a payload type** by what the event can *prove* about its host:
   `RemotePackageEvent` (an operation impossible without a remote),
   `PackageEvent` (a package that may be local-only), `PackageFileEvent`,
   `AuthEvent` / `AutosyncEvent` (host guaranteed), or no payload at all for
   app-lifecycle and debug actions.
3. **Add the arm to `MixpanelEvent::host()`.** It is exhaustive on purpose, so
   the compiler makes you decide whether your event names a deployment.
4. **Emit through the command helper**, not directly:

   ```rust
   Notify::new(msg_init)
       .on_success(&tracing, MixpanelEvent::PackagePulled(RemotePackageEvent::for_uri(uri.as_ref())))
       .map(result, msg_ok, msg_err)
   ```

Registering the event *before* the operation runs keeps the call site in the
order things happen, while emission waits for the outcome. This is what makes
"an event means the thing happened" structural rather than a convention: there
is no path to emission except the success arm.

**No free text, ever.** No package names, no paths, no user identity, no error
strings. A category is what a report can act on: "how often does a role denial
stop background sync" does not need to know which file. When a coarse category
is needed, follow `PausedKind` — an enum with an exhaustive `From` impl, so a
new upstream variant cannot inherit an answer.

**Failures are not events you add.** A refused action already reports itself:
the command helper classifies the error and emits `action_refused` with the
action's own wire name and a coarse reason. Add nothing for the failure path.

## Reporting a fault

`Telemetry::report_anomaly(&str)` for something that should not have happened
with no `Err` to carry it, and `report_error(&dyn Error)` for a failure the
caller is not failing on. Both go through the `Faults` seam rather than the SDK
directly, which is what lets a test assert that a path reported.

An anomaly's message **must be constant** — the crash reporter groups by it, so
a variable part belongs in a tag and not in the text, or one anomaly becomes one
issue per host. (A UI panic is the deliberate exception: its message varies so
that distinct panics become distinct issues.)

Frontend panics are bridged: the WASM panic hook calls `report_ui_panic`, so
they reach the log file and the crash reporter instead of only the browser
console. They cannot be *recovered* from — the runtime that would render an
error page is the one that died.

## What leaves the machine

Only from a release build, and only these:

- **Events**: name, coarse properties, the catalog host where one applies, a
  timestamp, an idempotency key, and the install id.
- **Faults**: message or error text, a stack trace, the breadcrumb trail
  (WARN/INFO events), the release version, the host tag, and the install id as
  the user id.
- **Nothing else.** The log file never leaves on its own — it goes only when a
  user chooses *Save diagnostics* and sends the archive.

**The install id** is a random UUID in `<app-data>/install_id`, generated on
first run and derived from nothing — not from an email, not from a hash of one.
It identifies an install, meaning one OS user on one machine. Deleting the file
makes the app a new install, which is also the cheapest shape for an opt-out to
reuse later.

**Undelivered events survive.** A send that fails with no verdict — offline,
timeout — keeps its events in `<app-data>/unsent_events.jsonl`, one per line,
bounded at 1000 with the oldest dropped. They replay on the next send that
succeeds. A *refused* send is not kept: the API answered, so a retry earns the
same answer. Nothing is flushed at exit: exit-time work exists (the log drains
there, above), but a send is a network round trip and a quit must not wait on one.

## Verifying a change without credentials

A debug build dry-runs both off-machine sinks to the terminal, which is the
whole verification loop for anything about *what* is reported:

```bash
just start
# analytics:
#   telemetry(dry-run) package_pulled {"host":"example.quiltdata.com","distinct_id":"…"}
# faults:
#   telemetry(dry-run) anomaly: <message>
```

What it cannot tell you is whether the ingest API *accepts* the payload — that
needs a real project. The client is configured with `verbose: true` so a
rejected event returns an error rather than a silent HTTP 200, and a refusal is
reported as a fault once per run.

<!-- markdownlint-disable MD013 -->
# Screens & Flows

ASCII wireframes for Quilt Sync's UI pages and user journeys.

## Pages

### Setup

First-run screen. User picks the home directory for Quilt data.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|                                                         |
|   Quilt stores packages in a local directory.           |
|                                                         |
|   Directory                                             |
|   [ /home/user/quilt________________ ]                  |
|   hint text                                             |
|                                                         |
|   [Browse...]                                           |
|                                                         |
+---------------------------------------------------------+
```

After submit -> **Installed Packages List**

---

### Login

Shown when authentication is required. Redirects back to the
page that triggered it via `back` parameter.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|                                                         |
|   [Login with Browser]                                  |
|                                                         |
|   ------------------------------------------------      |
|                                                         |
|   Or paste a code from the catalog:                     |
|   [Open Catalog]                                        |
|                                                         |
|   Code                                                  |
|   [ ________________________________________ ]          |
|                                                         |
+---------------------------------------------------------+
```

After login -> redirect to `back` URL (the page user came from)

---

### Installed Packages List (Home)

Main screen. Lists all locally installed packages.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages]                  [+ Create local package]  |
+---------------------------------------------------------+
|                                                         |
|  +---------------------------------------------------+  |
|  | user/package-a                        [Pull] [>]  |  |
|  +---------------------------------------------------+  |
|  | user/package-b              [Commit and Push] [>] |  |
|  +---------------------------------------------------+  |
|  | org/dataset-c                         [Pull] [>]  |  |
|  +---------------------------------------------------+  |
|  | local/my-data               [Set Remote] [>]      |  |
|  +---------------------------------------------------+  |
|                                                         |
+---------------------------------------------------------+
```

Packages where the host or bucket is missing (local-only with no
`manifest_uri`, or a partial remote) show `[Set Remote]` in the
context menu but no Commit-and-Push / Pull buttons and no "Open
Remote" action. The context menu never shows a `Change remote`
entry — editing a fully-configured remote is a detail-page concern,
handled by the toolbar button on the Installed Package page. After
setting a remote, the status changes to Ahead and `[Commit and Push]`
becomes available immediately (no re-commit needed).

`[Commit and Push]` is the one-click "commit changes (if any) then
push" action. It is shown when the package has a remote and
something to ship — uncommitted changes, a pending commit that was
not yet pushed, or both. It uses per-user defaults from Settings
for message, workflow, and metadata; users who need a bespoke
message enter the full form via the package page's `[Commit…]`
link.

Empty state:

```text
+--[toolbar]---------------------------------------------+
| [< Packages]                  [+ Create local package]  |
+---------------------------------------------------------+
|                                                         |
|   No packages installed                                 |
|   Install packages from the catalog using deep links.   |
|                                                         |
|   [how-to-deep-link illustration]                       |
|                                                         |
+---------------------------------------------------------+
```

- Click [+ Create] -> opens **Create Package** popup
- Click package row -> **Installed Package**
- Click [Pull] -> runs pull flow, reloads
  - Two-phase: enabled state and copy fill in from a dry-run pull check;
    disabled only while checking, if the check fails (Retry offered), or
    when the outcome is a conflict (see the status banner below)
- Click [Commit and Push] -> commits if needed, then pushes;
  reloads
  - Disabled while the package's status is still refreshing in the
    background
  - Never shown without a remote; `[Set Remote]` appears instead
- Click [Set Remote] -> opens **Set Remote** popup
- Click [gear] -> **Settings**

Rows update live from the background autopull watcher: when a
tick changes a package's `UpstreamState` or pauses / un-pauses a
namespace, the row's status pill and contextual button refresh
without a page reload. Status changes propagate via
`PackageStatusEvent`; pause / un-pause via `AUTOSYNC_PAUSED_EVENT`.

---

### Installed Package

Shows contents of a single installed package: file entries
with checkboxes, status indicator, and a toolbar.

```text
+--[appbar]----------------------------------------------+
| [logo]  user/package-a                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages]   [Open in File Browser] [Open in Catalog?] [Set Remote / Change Remote] [Uninstall] |
+---------------------------------------------------------+
| [status banner: status-dependent action ────────────]   |
|   ahead        -> [Push]                                |
|   behind       -> [Pull]   (see two-phase note below)   |
|   diverged     -> [Merge]                               |
|   local+origin -> [Push]                                |
|   error        -> [Login]                               |
|                                                         |
| Show [x] unmodified [x] ignored (2)                    |
| [toolbar: Select All / Deselect All]                    |
| +-----------------------------------------------------+ |
| | [x] data/file-a.csv            1.2 MB  M            | |
| | [x] data/file-b.parquet        3.4 MB  A            | |
| | [ ] data/file-c.json           0.5 MB      [Ignore] | |
| | [ ] data/subdir/file-d.txt     0.1 MB               | |
| |                                                      | |
| |  (dimmed)                                            | |
| | [ ] .DS_Store                  4 KB     [Ignored]    | |
| +-----------------------------------------------------+ |
|                                                         |
+--[actionbar]-------------------------------------------+
|        [Create new revision]  or  [Commit and Push]     |
+---------------------------------------------------------+
```

- [< Packages] -> **Installed Packages List**
- Top toolbar hosts package-agnostic actions: file browser, catalog,
  the remote-editing action (`Set remote` when host or bucket is missing,
  `Change remote` otherwise — both open the same popup), and Uninstall.
  Package sync actions (Push / Pull / Merge / Login) live in the status
  banner below it.
- [Push] -> runs push flow, reloads
- [Pull] -> two-phase. The `behind` banner renders immediately with a
  "Checking for updates…" placeholder and Pull disabled; a dry-run pull
  check then drives the copy and enabled state:
  - still loading -> disabled, "Checking for updates…"
  - check failed -> disabled, "Couldn't check for updates." + [Retry]
  - clean update -> enabled, "The remote has newer revisions."
  - keeps local changes -> enabled, "The remote has newer revisions.
    Your local changes are safe — pulling keeps them."
  - conflict -> disabled, "Conflicts in {files}. Commit your changes to
    resolve them on the merge page."
- [Uninstall] -> runs uninstall flow -> **Installed Packages List**
- [Install Selected Paths] -> runs install_paths flow, reloads
- [Ignore] -> opens **Ignore Popup** (for junk-detected files)
- [Ignored] -> opens **Un-ignore Popup** (for `.quiltignore`-matched files)
- [Create new revision] -> opens the **Commit** form at
  `/commit?namespace=…`; the form is where the user picks message,
  workflow, and metadata before either saving locally (`[Commit]`) or
  committing-and-pushing (`[Commit and Push]`)
- [Commit and Push] -> one-click commit-and-push using the
  Commit-and-Push defaults from Settings. Shown in the actionbar
  only when the package has a remote and something to ship
  (uncommitted changes or a pending commit that has not been pushed).
  Equivalent to the `[Commit and Push]` button on the Installed
  Packages List.

**Autosync paused (variant)**: when the background autopull watcher
refuses to act for a `PausedReason::Other(_)` reason (workflow
rejection, persistent push/commit error after backoff, etc.) or for a
`PausedReason::PullConflict` (autopull hit a two-sided conflict), an
additional banner stacks above the standard status banner:

```text
+--[autosync paused]-------------------------------------+
| Autosync paused: <message from PausedReason::Other(_)> |
|                                              [Dismiss] |
+--------------------------------------------------------+
```

- A `PullConflict` pause shows conflict-specific guidance instead of
  the generic text: "Conflicts in {files}. Commit your changes to
  resolve them on the merge page." — the same remediation the manual
  pull's `Blocked` banner gives.
- Suppressed for `Diverged`, `Behind`, and pending-change/commit paused
  reasons — the standard status banner already surfaces those with their
  own action (`[Merge]` / `[Pull]`).
- Re-hydrated on page mount from `get_autosync_snapshot()`; live
  updated via the `AUTOSYNC_PAUSED_EVENT` Tauri event.
- `[Dismiss]` only hides the banner locally; the underlying
  refusal is cleared when the next watcher tick succeeds or when
  the user resolves the upstream condition.

---

### Commit

Form for committing local changes to a package.
Two-column layout: form on the left, file list on the right.

```text
+--[appbar]----------------------------------------------+
| [logo]  user/package-a                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages]                                            |
+---------------------------------------------------------+
|                        |                                |
|  [workflow indicator]  | Show [x] unmodified            |
|                        |       [x] ignored (1)          |
|  Namespace             | -------------------------      |
|  [ user/package-a   ]  |  data/file-a.csv          M   |
|  (readonly)            |  data/file-b.parquet      A   |
|                        |  data/file-c.json         D   |
|  Message *             | -------------------------      |
|  [ Updated dataset__ ] |  .DS_Store        [Ignored]   |
|                        |                                |
|  Metadata              |                                |
|  [ { "key": "value" }] |                                |
|  [rendered metadata]   |                                |
|                        |                                |
+---------------------------------------------------------+
+--[actionbar]-------------------------------------------+
|                     [Commit]  or  [Commit and Push]     |
+---------------------------------------------------------+
```

Both buttons are disabled when the message field is empty.

- `[Commit and Push]` is the primary action — it commits the form
  values and immediately pushes the new revision.
- `[Commit]` stays available as a secondary action for users who want
  to save a local revision without pushing (e.g. to squash with a
  later commit before pushing).
- For local-only packages (no remote configured) only `[Commit]` is
  shown, because there is nothing to push to.

After either action -> **Installed Package**

---

### Merge

Shown when local and remote versions have diverged.
Offers two resolution options.

```text
+--[appbar]----------------------------------------------+
| [logo]  user/package-a                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages] > [package-a]              [Push] [Pull]  |
+---------------------------------------------------------+
|                                                         |
|   Certify your local version as the latest:             |
|   [Certify as Latest]                                   |
|                                                         |
|   Or discard local changes and reset to remote:         |
|   [Reset to Remote]                                     |
|                                                         |
+---------------------------------------------------------+
```

- [Certify as Latest] -> runs certify_latest flow
- [Reset to Remote] -> runs reset_to_latest flow

---

### Ignore Popup

Shown when clicking [Ignore] on a junk-detected file entry.
Lets the user add a pattern to `.quiltignore`.

```text
+---------------------------------------------------------+
|                                                         |
|   Pattern                                               |
|   [ *.pyc______________________________ ]               |
|   hint: will be ignored                                 |
|     or: Ignore all similar files with `*.pyc`           |
|                                                         |
|   [Add to .quiltignore]  [Cancel]                       |
|                                                         |
+---------------------------------------------------------+
```

- Pattern field is pre-filled with the suggested pattern
- Live validation with debounce shows what the pattern matches
- [Add to .quiltignore] -> appends pattern, reloads page

---

### Un-ignore Popup

Shown when clicking [Ignored] on a `.quiltignore`-matched file.
Shows which pattern is ignoring the file.

```text
+---------------------------------------------------------+
|                                                         |
|   Ignored by pattern: *.pyc                             |
|                                                         |
|   [Edit .quiltignore]  [Cancel]                         |
|                                                         |
+---------------------------------------------------------+
```

- [Edit .quiltignore] -> opens file in default application

---

### Create Package Popup

Shown when clicking [+ Create] on the Installed Packages List page.
Creates a new local-only package.

```text
+---------------------------------------------------------+
|                                                         |
|   Namespace *                                           |
|   [ owner/package-name________________ ]                |
|                                                         |
|   [Create]  [Cancel]                                    |
|                                                         |
+---------------------------------------------------------+
```

- Namespace must be `owner/name` format
- After create -> page reloads, new package appears in list
- Package starts as local-only with an initial empty commit

---

### Set Remote Popup

Shown from the Installed Package toolbar on any package, and from the
Installed Packages List context menu when host or bucket is missing.
Both `[Set remote]` and `[Change remote]` labels open the same popup.
Host and bucket are pre-filled from the current remote state (empty
strings when unset) so editing a partial or mistyped remote is just
correcting the input. Title is always "Set remote".

```text
+---------------------------------------------------------+
|                                                         |
|   Set remote                                            |
|                                                         |
|   Host *                                                |
|   [ open.quiltdata.com________________ ]                |
|                                                         |
|   Bucket *                                              |
|   [ my-s3-bucket______________________ ]                |
|                                                         |
|   [Save]  [Cancel]                                      |
|                                                         |
+---------------------------------------------------------+
```

- After save -> page reloads, status changes to Ahead
- Push becomes available immediately (no re-commit needed)

---

### Edit Commit and Push Defaults Popup

Shown when clicking `[Edit]` in Settings → Commit and Push. Form
for the global defaults the one-click `[Commit and Push]` action
uses for every package (no per-package overrides).

```text
+---------------------------------------------------------+
|  Edit Commit and Push defaults                          |
|                                                         |
|  Message template                                       |
|  [ Auto-publish {date} ({changes})_________________ ]   |
|  Placeholders: {date} {time} {datetime}                 |
|                {namespace} {changes}                    |
|  Preview: Auto-publish 2026-04-21 (3 files modified)    |
|                                                         |
|  Default workflow                                       |
|   (*) Use the bucket's default workflow                 |
|   ( ) Override:  [ __________________________ ]         |
|                                                         |
|  Default metadata                                       |
|  [ { "source": "desktop" }_________________________ ]   |
|                                                         |
|  [Save]  [Cancel]   Reset to defaults                   |
|                                                         |
+---------------------------------------------------------+
```

- Preview re-renders live as the user types (client-side, no round
  trip); unknown placeholders like `{dat}` pass through so typos stay
  visible.
- Workflow radio: "Use bucket's default workflow" sends no workflow
  id at Commit-and-Push time; "Override" saves the typed id,
  validated against the bucket on the first run (same as today's
  Commit form).
- Metadata is JSON-validated on Save; empty = no metadata.
- `Reset to defaults` clears all three fields but does not save —
  user still has to click `[Save]`.
- After save -> popup closes, Settings page reloads showing the new
  values.

---

### Edit Autosync Settings Popup

Shown when clicking `[Edit]` in Settings → Autosync.
Form for the two direction toggles, the pull interval, and the
publish quiet window.

```text
+---------------------------------------------------------+
|  Edit autosync settings                                 |
|                                                         |
|  [ ] Auto-pull updates from the remote                  |
|  hint: pull when remote moves ahead and local is clean  |
|                                                         |
|  [ ] Auto-publish local changes                         |
|  hint: commit and push once the working tree is quiet   |
|                                                         |
|  Pull interval (seconds)                                |
|  [ 30 ]                                                 |
|                                                         |
|  Wait after last edit before publishing (seconds)       |
|  [ 300 ]                                                |
|                                                         |
|  [Save]  [Cancel]   Reset to defaults                   |
|                                                         |
+---------------------------------------------------------+
```

- The two checkboxes are independent: many users want background
  pulls without unattended pushes. Either toggle being on starts
  the watcher; both off stops it.
- Intervals are positive integers; the form rejects non-numeric
  or zero input before saving, and the Tauri command rejects zero
  again as a backstop.
- Auto-publish refuses on `Diverged` and on foreign-remote
  conflicts — those still require explicit user action via
  **Merge** or **Set Remote**.
- `Reset to defaults` restores the shipped defaults (both off,
  30 s pull interval, 300 s wait) but does not save — user still
  has to click `[Save]`.
- After save -> popup closes, watcher cadence updates immediately,
  Settings page reloads.

---

### Settings

Application settings and diagnostics.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|                                                         |
|  General                                                |
|  -------                                                |
|  Version          0.27.0  [Release Notes]               |
|  Home directory   /home/user/quilt  [Open]              |
|  Data directory   /home/user/.quilt [Open]              |
|                                                         |
|  Commit and Push                                        |
|  ---------------                                        |
|  Message template   Default — auto-generated summary    |
|  Default workflow   Default — bucket's workflow         |
|  Default metadata   Default — none                      |
|                                                         |
|  [Edit]                                                 |
|                                                         |
|  Autosync                                               |
|  --------                                               |
|  Pull (remote -> local)        Off                      |
|  Push (local -> remote)        Off                      |
|  Pull interval                 30 s                     |
|  Wait after last edit          300 s                    |
|                                                         |
|  [Edit]                                                 |
|                                                         |
|  Filesystem Watcher                                     |
|  ------------------                                     |
|  Enable filesystem watcher     [x]                      |
|  hint: refreshes local status when files change         |
|                                                         |
|  Account                                                |
|  -------                                                |
|  open.quiltdata.com       [Re-login] [Logout]           |
|  custom.registry.io       [Re-login] [Logout]           |
|                                                         |
|  Diagnostics                                            |
|  -----------                                            |
|  Log level        INFO                                  |
|  Logs directory   /home/user/.quilt/logs  [Open]        |
|                                                         |
|  [Collect Logs] then [Send Crash Report]                |
|                  or   [Email Support]                    |
|                                                         |
+---------------------------------------------------------+
```

---

### Error

Generic error page with recovery options.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|                                                         |
|   Error Title                                           |
|                                                         |
|   Detailed error message explaining what went wrong.    |
|                                                         |
|   [Reload] [Open .quilt] [Login] [Home]                 |
|                                                         |
+---------------------------------------------------------+
```

Buttons shown depend on context (e.g. [Login] only when
auth-related).

---

## Flow Diagram

```text
                        +-------+
                        | Setup |  (first run only)
                        +---+---+
                            |
                            v
  +---------------------------------------------------+
  |          Installed Packages List (Home)            |<---------+
  +---+-------+------------+---------------+----------+           |
      |       |            |               |                      |
      |       | [Create]   | [Commit and   | [gear]               |
      |       v            |     Push]     v                      |
      |  [popup]           v          +----------+                |
      |  namespace    (commit if      | Settings |                |
      |  -> reload    needed, then    +----------+                |
      |               push) -> reload                             |
      |                                                           |
      | click pkg                                                 |
      v                                                           |
+-----+-------+      [Push]        (status banner: ahead / local) |
|  Installed   |----> push only -----------------------> reload --+|
|  Package     |                                                 ||
|              |      [Commit and Push]   (bottom actionbar)     ||
|              |----> commit+push (uses Settings defaults) ----->+|
|              |                                                 ||
|              |      [Create new revision]  (bottom actionbar)  ||
+--+-+--+------+----> +----+----+                                ||
   | |  |             |         |                                ||
   | |  |             | Commit  |                                ||
   | |  |             |         |                                ||
   | |  |             +----+----+                                ||
   | |  |                  |                                     ||
   | |  |                  | [Commit and Push]  (primary)        ||
   | |  |                  | [Commit]           (secondary)      ||
   | |  |                  v                                     ||
   | |  |             --->--+-------------------------->---------+|
   | |  |                                                         |
   | |  |   [Set remote] / [Change remote]  (toolbar and list    |
   | |  |                                    context menu)        |
   | |  |     -> popup (host + bucket, pre-filled from state)     |
   | |  |     -> status updates on save -> reload                 |
   | |  | [Uninstall]                                             |
   | |  +---->----------------------------------------------------+
   | |
   | | when diverged
   | v
   | +---------+
   +>|  Merge  |
     +---------+
     | Certify | Reset
     +----+----+
          |
          v
     (back to Installed Package)


  Any page ----[needs auth]----> Login ----[back]----> original page
```

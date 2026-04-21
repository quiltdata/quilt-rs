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
|  | user/package-b                     [Publish] [>]  |  |
|  +---------------------------------------------------+  |
|  | org/dataset-c                         [Pull] [>]  |  |
|  +---------------------------------------------------+  |
|  | local/my-data               [Set Remote] [>]      |  |
|  +---------------------------------------------------+  |
|                                                         |
+---------------------------------------------------------+
```

Local-only packages (no remote `manifest_uri`) show `[Set Remote]`
but no Publish/Pull buttons and no "Open Remote" action. After
setting a remote, the status changes to Ahead and Publish becomes
available immediately (no re-commit needed).

`[Publish]` is the one-click "commit changes (if any) then push"
action. It is shown when the package has a remote and something to
ship — uncommitted changes, a pending commit that was not yet
pushed, or both. Publish uses per-user defaults from Settings for
message, workflow, and metadata; users who need a bespoke message
enter the full form via the package page's `[Commit…]` link.

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
  - Disabled with popover hint when package has uncommitted changes
- Click [Publish] -> runs publish flow (commit if needed, then push),
  reloads
  - Disabled while the package's status is still refreshing in the
    background
  - Never shown without a remote; `[Set Remote]` appears instead
- Click [Set Remote] -> opens **Set Remote** popup
- Click [gear] -> **Settings**

---

### Installed Package

Shows contents of a single installed package: file entries
with checkboxes, status indicator, and a toolbar.

```text
+--[appbar]----------------------------------------------+
| [logo]  user/package-a                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages]   [Open in File Browser] [Open in Catalog?] [Uninstall] |
+---------------------------------------------------------+
| [status banner: status-dependent action ────────────]   |
|   ahead        -> [Push]                                |
|   behind       -> [Pull]   (disabled if local changes)  |
|   diverged     -> [Merge]                               |
|   local+origin -> [Push]                                |
|   error        -> [Login] / [Set Origin] / [Change…]    |
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
- Top toolbar hosts package-agnostic actions only: file browser, catalog,
  and Uninstall. Package sync actions (Push / Pull / Merge / Login)
  live in the status banner below it.
- [Push] -> runs push flow, reloads
- [Pull] -> disabled with popover hint when package has uncommitted changes
- [Uninstall] -> runs uninstall flow -> **Installed Packages List**
- [Install Selected Paths] -> runs install_paths flow, reloads
- [Ignore] -> opens **Ignore Popup** (for junk-detected files)
- [Ignored] -> opens **Un-ignore Popup** (for `.quiltignore`-matched files)
- [Create new revision] -> opens the **Commit** form at
  `/commit?namespace=…`; the form is where the user picks message,
  workflow, and metadata before either saving locally (`[Commit]`) or
  committing-and-pushing (`[Commit and Push]`)
- [Commit and Push] -> one-click commit-and-push using the Publish
  defaults from Settings. Shown in the actionbar only when the package
  has a remote and something to ship (uncommitted changes or a pending
  commit that has not been pushed). Equivalent to `[Publish]` on the
  Installed Packages List — surfaced here as a CTA labeled to match the
  Commit form.

For local-only packages without an origin, the status banner shows
[Set Origin] instead of Push.

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
  later commit before publishing).
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

Shown when clicking [Set Remote] on a local-only package.
Configures the remote origin and bucket so the package can be pushed.

```text
+---------------------------------------------------------+
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

### Edit Publish Defaults Popup

Shown when clicking `[Edit]` in Settings → Publish. Form for the
global defaults the one-click `[Publish]` action uses for every
package (no per-package overrides).

```text
+---------------------------------------------------------+
|  Edit publish defaults                                  |
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
  id at publish time; "Override" saves the typed id, validated
  against the bucket on the first Publish (same as today's Commit
  form).
- Metadata is JSON-validated on Save; empty = no metadata.
- `Reset to defaults` clears all three fields but does not save —
  user still has to click `[Save]`.
- After save -> popup closes, Settings page reloads showing the new
  values.

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
|  Publish                                                |
|  -------                                                |
|  Message template   Default — auto-generated summary    |
|  Default workflow   Default — bucket's workflow         |
|  Default metadata   Default — none                      |
|                                                         |
|  [Edit]                                                 |
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
      |       | [Create]   | [Publish]     | [gear]               |
      |       v            v               v                      |
      |  [popup]      (commit if           +----------+           |
      |  namespace    needed, then         | Settings |           |
      |  -> reload    push) -> reload      +----------+           |
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
   | |  |   [Set Remote]                                          |
   | |  |     -> popup (host + bucket)                            |
   | |  |     -> status: Ahead -> Publish                         |
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

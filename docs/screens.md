<!-- markdownlint-disable MD013 -->
# Screens & Flows

ASCII wireframes for Quilt Sync's UI pages and user journeys.

## Pages

### Setup

First-run screen, at `/setup`. One card with the explanation, the directory
field (chosen with **Browse**, not typed) and **Save**. A failed pick or save
says so in the band under the appbar.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|        Set up QuiltSync                                 |
|        Select a directory where QuiltSync will ...      |
|        Set home directory                               |
|        [ /home/user/QuiltSync__________ ] [Browse]      |
|        [Save]                                           |
+---------------------------------------------------------+
```

After save -> **Main Page**

---

### Login

Shown when authentication is required, at `/login`. It returns to the page
that sent it via the `back` parameter. One card titled with the host. **Log
in with browser** first; then the catalog's address with **Open browser**, the
**Code** field and **Submit code and Log in**, which waits for a code. A failure says so in the band under the
appbar; opening the browser sign-in says to continue there.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|        Log in to open.quiltdata.com                     |
|        [Log in with browser]                            |
|        ------------------------------------------       |
|        Or visit https://open.quiltdata.com/code ...     |
|        [Open browser]                                   |
|        Code                                             |
|        [ ______________________________ ]               |
|        [Submit code and Log in]                         |
+---------------------------------------------------------+
```

After login -> redirect to `back` URL (the page user came from)

---

### Main Page

The main screen, at `/`.

Four regions, in reading order. The queue is absent — not empty — when nothing
needs a decision, and says so in one line instead.

```text
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|  +--[Autosync]--------+  +--[Accounts]-----------------+ |
|  | Pulling      [x]   |  | open.quiltdata.com          | |
|  | Publishing   [x]   |  |   as ReadWriteQuiltBucket   | |
|  | Next check in 0:28 |  | demo.quiltdata.com          | |
|  +--------------------+  |   Signed out    [Sign in]   | |
|                          +-----------------------------+ |
+---------------------------------------------------------+
|  ▲ 4 packages need your attention                       |
|  ------------------------------------------------------ |
|  > Signed out from demo.quiltdata.com — 2 packages      |
|                                              [Sign in]  |
|  ------------------------------------------------------ |
|    user/pkg-a  has a newer revision        Get latest → |
|  ------------------------------------------------------ |
|    user/pkg-b  has 2 changed files            Publish → |
|  ------------------------------------------------------ |
+--[toolbar]----------------------------------------------+
| [Packages|Recent files]  [Search…] [Group:Bucket]        |
|                          [Sort:Changed] [Create package] |
+---------------------------------------------------------+
|  +---------------------------------------------------+  |
|  | user/package-a          Latest       2 hours ago   |  |
|  | user/package-b          2 files changed   just now |  |
|  +---------------------------------------------------+  |
+---------------------------------------------------------+
```

- **Autosync card** — whether background pulling and publishing are running,
  each switchable on its own, and when the next check is due.
- **Accounts card** — every catalog your packages point at plus every host you
  have signed into, with the role each is using.
- **Attention queue** — a status line, `N packages need your attention`, then
  one row per package needing a decision, beside the one thing to do about it.
  Where a whole catalog or bucket is the cause, it is said once with a count
  rather than repeated per package. When nothing needs you, the line alone:
  `All packages are up to date`. Until every package's check has answered, a
  placeholder holds that line's place, so a healthy load does not move the
  list.
- **List** — your packages with each one's state and when it last changed,
  searchable, groupable by Bucket / Prefix / None and sortable by Changed /
  Name; or switched to a feed of recently changed files.

Grouping options differ per view: packages group by Bucket, Prefix or None;
recent files group by Package or None.

Resting the pointer anywhere on a package row, or reaching it with Tab, opens
its tooltip; the pointer passes through it to the rows below. It gives:

- the package's full name, which the row may cut short;
- *Host: open.quiltdata.com*, left out for a package with no catalog;
- what it keeps: *Every file in the package is kept*, *Only the files you
  picked are kept*, or *Files in 2 folders are kept*;
- how much is downloaded, in Keeping's words on the package page, as in *"12 of
  340 files · 1.9 MB of 3.4 MB downloaded."*

The last two are left out until the row's check answers, and again when a check
fails.

- Click a package row -> **Installed Package**
- A queue row's action (`Get latest`, `Publish`, `Choose S3 bucket`, ...)
  navigates; it never runs the command itself. Each lands on the control on
  the package page that does.
- [Create package] -> opens the **Create Package Dialog**
- [gear] -> **Settings**

---

### Installed Package

One installed package, at `/installed-package`: a header, a file pane and a
context pane.

```text
+--[appbar]-------------------------------------------------------------------+
| [logo]                                                     [refresh] [gear] |
+-----------------------------------------------------------------------------+
| < Packages                                                                  |
| user/package-a  [Newer revision available]                  [Get latest]    |
|                     [Open folder] [Open in catalog] [Copy Quilt URI]  [...] |
+-----------------------------------------------------------------------------+
| [Search files...]                              | Bucket and catalog         |
| [ ] Select all     [Group v] [All | Changed |  | Revision                   |
|                    Not downloaded | Ignored]   | Keeping                    |
| raw/ 54                                        |                            |
|   [v] data/file-a.csv               Changed    |                            |
|   [ ] data/file-b.parquet    Not downloaded    |                            |
| [Download 3 · 114.1 kB]                        |                            |
+-----------------------------------------------------------------------------+
```

- **Header** — the trail back to **Main Page**, the package's name, its one
  state, and that state's one action: `Publish` (its caret holds `Review before
  publishing…`, which opens the **Commit** page), `Get latest`, `Resolve` or
  `Sign in`. Then the location group: *Open folder*, *Open in catalog*,
  *Copy Quilt URI*. Everything else is behind `[⋯]`: *Create new revision*
  (opens **Commit**), *Undo last revision* and *Remove*, each disabled with
  its reason when it cannot run. *Remove* asks first, with *Also delete
  downloaded files from disk*, then goes to **Main Page**.
- **File pane** — the package's files. Search, select-all, grouping, and the
  facets *All · Changed · Not downloaded · Ignored*, each with the package's
  count. A row's menu holds *Ignore*, which opens the **Ignore Popup**, or on
  an ignored row *Stop ignoring*, which opens the **Stop Ignoring Popup**.
- **Context pane** — *Bucket and catalog*, which opens the **Bucket and
  Catalog Dialog**; the current *Revision*, with the revisions this copy holds;
  and *Keeping*.

The context pane has two other modes, each asked for by the address:

- **Resolve** (`resolve=1`), for a package whose revision and the published
  one have diverged: *Yours* against *Published*, the files that differ marked
  in the file pane, and two choices. **Share mine**: "Everyone who syncs gets
  your revision next. Nothing is deleted." **Replace mine** swaps your
  revision and edits here for the published one, after a confirmation that
  says what it overwrites.
- **Revision** (`revision=<hash>`) — the current revision whole, with its hash,
  publication, workflow and full metadata.

The context pane's **Keeping** caption says how much of
the current revision is here, in files and in bytes:

- *"54 of 56 files · 1.9 MB of 3.4 MB downloaded."*
- *"All files are downloaded · 3.4 MB."* when nothing is outstanding, and
  *"No files are downloaded · 3.4 MB."* when nothing is here.
- *"This revision has no files."* for an empty revision.
- Under *The whole package* the caption adds *"— files added later are
  downloaded too."*, and *Download 2 files · 1.5 MB* fetches the backlog.
- Under *Folders* the kept folders are listed with *Change folders*, which
  opens a dialog of folder paths, one a row, with *Add folder*, *Cancel* and
  *Save*. A folder keeps every file under it, including files added there
  later; `data/raw` keeps `data/raw/x/a.csv` and not `data/raw2/a.csv`.
  Choosing *Folders* with none kept opens the dialog, and *Cancel* returns to
  the previous option. *Save* downloads nothing: *Download N files* fetches
  the files under the kept folders. A folder with `..` is refused in the
  dialog. Choosing another option replaces the folders. The CLI sets the same
  folders with `quilt keep --folder`. In the file list, files under the kept
  folders have no checkbox, and *Get latest* downloads the new ones there.

The sizes are the manifest rows' sizes summed, not disk usage. Only the files
the current revision lists are counted: a new file is not counted until it is
committed, and a modified file counts at the size the manifest records, not its
size on disk. A file deleted here counts as downloaded: it is a local change
waiting to commit, and Download would not fetch it. So the caption names those
files, as in *"54 of 56 files · 1.9 MB of 3.4 MB downloaded · 1 deleted here."*
When the sizes cannot be read the caption gives the count alone, *"54 of 56
downloaded."*, and the button *Download 2 files*. The file pane's footer says
the ticked files' bytes the same way, *Download 3 · 114.1 kB*.

---

### Commit

Form for a new revision, at `/commit`. The header and any banner span the
page. From about a 1024px window, the form is on the left and the changed
files on the right; narrower, it is one column with the files last.

```text
+--[appbar]-------------------------------------------------------------------+
| [logo]                                                     [refresh] [gear] |
+-----------------------------------------------------------------------------+
| Packages > user/package-a                                                   |
| New revision                         [Publish 3 files | v] [Open folder]    |
|                                                                             |
| [! 2 system files would be published (.DS_Store)]           [Ignore them]   |
|                                                                             |
| Message                              What's included · 3 files · 8.5 MB     |
|                                                2 ignored files not included |
| [ Updated 3 files: a.csv, ... ]      +-----------------------------------+  |
| Workflow                             | data/file-a.csv  Changed 4.2 MB ..|  |
| [ Bucket's default (Plate reads) v ] | data/file-b.parq New     4.3 MB ..|  |
| Metadata  Edit                       | data/file-c.json Deleted 18 kB  ..|  |
| v {                                  +-----------------------------------+  |
|     assay: "ELISA"                                                          |
|     plate: 7                                                                |
|   }                                                                         |
+-----------------------------------------------------------------------------+
```

- The rows scroll in their own box, so the header and its primary stay on
  screen however long the list is. Wide, the box fills the right column.
  Narrow, it takes the height the form leaves, with a floor of two rows and a
  half; when the form leaves less (the editor open in a short window), the
  whole page scrolls instead.
- The rows have no checkboxes, so they have no leading column either.
- With no file changes, the right column shows a file glyph over "No file
  changes. This revision only updates metadata."
- The primary reads `Publish N files`, or `Publish revision` when only
  metadata changed; its caret holds `Save without publishing`. A package
  with no bucket gets `Save revision` with no caret. N and the size count
  every change in the package, not the capped list; the size is the new and
  changed files' bytes. The list holds at most 1000 rows (the package page's
  cap), changes first; past it a line says "Showing 1000 of N files. All N
  are included." The system-files banner counts the whole package.
- The message starts empty over the publish message as its placeholder;
  an empty message submits the placeholder.
- The workflow is a live select, or plain words when there is nothing to
  choose. The metadata is a read-only preview, one line per top-level field,
  with `Edit` (`Add` when empty) right after its name, which opens the JSON
  editor and becomes `Close editor`; a failed metadata check opens it.
  The bar under the editor drags it taller, and never shorter than it
  opens. Each says "From your publish settings" only while Settings supplied it.
- A role without access, no session, or a failing workflow check disables
  the whole primary, with the reason in a tooltip.
- `Ignore` on a row and `Ignore them` on the banner open the **Ignore
  Popup**; adding the pattern re-reads the page.
- Arriving from `Create new revision` focuses the message
  (`focus=message`); `Review before publishing…` focuses nothing.

After either action -> **Installed Package**. A failure stays on the page,
in the banner under the appbar, and the page re-reads.

---

### Ignore Popup

Shown from *Ignore* on a file row, or *Ignore them* on the commit page's
system-files banner. Adds a pattern to `.quiltignore`.

```text
+---------------------------------------------------------+
|                                                         |
|   Pattern to ignore:                                    |
|   [ *.pyc______________________________ ]               |
|   data/file.pyc will be ignored                         |
|                                                         |
|   [Add to .quiltignore]  [Cancel]                       |
|                                                         |
+---------------------------------------------------------+
```

- Pattern field is pre-filled with the suggested pattern
- As the pattern is typed, the hint says what it matches: the path *will be
  ignored*, *Only* the path will be, or it *Doesn't match* the path
- [Add to .quiltignore] -> appends pattern, re-reads the page

---

### Stop Ignoring Popup

Shown from *Stop ignoring* on an ignored file row. Says which pattern ignores
the path, and brings it back.

```text
+---------------------------------------------------------+
|                                                         |
|   Stop ignoring data/file.pyc                           |
|   Ignored by: *.pyc                                     |
|                                                         |
|   Stop ignoring in                                      |
|   (*) This package   ( ) All packages                   |
|   This adds 1 file (4 kB) to the package on the next    |
|   sync.                                                 |
|   Adds !data/file.pyc to this package's .quiltignore    |
|                                                         |
|   [Stop ignoring]  [Cancel]                             |
|                                                         |
+---------------------------------------------------------+
```

- *This package* is chosen when the popup opens; *All packages* is offered
  only when a user-wide line can bring the path back
- While the files are counted, and when counting failed, [Stop ignoring] is
  disabled
- [Stop ignoring] -> writes the `!` lines, re-reads the page

---

### Create Package Dialog

Shown from [Create package] in the Main Page's toolbar. Creates a new
local-only package.

```text
+---------------------------------------------------------+
|  Create package                                         |
|                                                         |
|   Package name  Required                                |
|   [ owner/package-name________________ ]                |
|   Two parts, separated by a slash — user/plate-07.      |
|                                                         |
|   Folder to add                                         |
|   [ No folder chosen__________________ ] [Browse…]      |
|   Optional. You can add files later.                    |
|                                                         |
|                                     [Cancel]  [Create]  |
+---------------------------------------------------------+
```

- Package name must be `owner/name` format
- Without a desktop (headless) the folder is typed, not picked
- A failure shows as a banner above the fields
- After create -> the list re-reads, new package appears in it
- Package starts as local-only with an initial empty commit

---

### Bucket and Catalog Dialog

Shown from the package page's *Bucket and catalog* section, and from the
commit page's menu (*Change bucket*) before the first push. Before the first
push it sets host, bucket and workflow. A published package keeps its bucket:
the bucket is shown and disabled, there is no workflow, and Save changes only
the catalog.

```text
+---------------------------------------------------------+
|  Bucket and catalog                                     |
|                                                         |
|   Host                                                  |
|   [ open.quiltdata.com________________ ]                |
|                                                         |
|   Bucket                                                |
|   [ my-s3-bucket______________________ ]                |
|                                                         |
|   Workflow                                              |
|   [ Plate reads (default) v ]                           |
|                                                         |
|                                       [Cancel]  [Save]  |
+---------------------------------------------------------+
```

---

### Publishing Dialog

Shown from **Edit…** on Settings' *Publishing* card. The global defaults
publishing and autosync use for every package; they also fill in a new
revision's form.

```text
+---------------------------------------------------------+
|  Publishing                                             |
|                                                         |
|  Message template                                       |
|  [ Generated from the changes______________________ ]   |
|  Placeholders: {date} {time} {datetime}                 |
|                {namespace} {changes}                    |
|                                                         |
|  Default workflow                                       |
|  [ Each bucket's default___________________________ ]   |
|  A workflow id. Empty uses each bucket's default.       |
|                                                         |
|  Default metadata                                       |
|  [ { "source": "desktop" }_________________________ ]   |
|                                                         |
|  [Reset to defaults]                  [Cancel]  [Save]  |
+---------------------------------------------------------+
```

- Save is off while the metadata is not valid JSON.
- A named workflow or any metadata shows a warning: these settings apply to every
  bucket, so a default can make publishing fail in a bucket that doesn't
  define it.
- *Reset to defaults* clears the fields but saves nothing, so Cancel undoes
  it.

---

### Quit Confirmation

Shown when quitting would interrupt a package being written — the tray's
**Quit**, or closing the window when close-to-tray is off. Not shown when
nothing is being written, which is the common case.

```text
+---------------------------------------------------------+
|                                                         |
|   Still syncing                                         |
|                                                         |
|   QuiltSync is writing files right now. Quitting will    |
|   interrupt it and leave this package part-way between   |
|   two revisions.                                        |
|                                                         |
|   [Stay]  [Quit anyway]                                 |
|                                                         |
+---------------------------------------------------------+
```

- Raised over whatever page is open; the tray path shows the window first
- No click-outside dismiss — the two buttons are the only ways out
- [Stay] -> abandons the quit; a later quit asks again
- [Quit anyway] -> exits, interrupting the write
- It asks, it never waits for the write to finish
- If the prompt cannot be shown or does not report itself on screen, the quit
  proceeds rather than appearing to be ignored

---

### Settings

Application settings and diagnostics, at `/settings`. Cards in rows of two
on a wide window, in the same order in one column when narrow. No Refresh or
Settings in the appbar.

```text
+--[appbar]----------------------------------------------+
| [logo]                                                  |
+---------------------------------------------------------+
| < Packages                                              |
| Settings                                                |
|                                                         |
| Syncing                                                 |
|  [x] Get new revisions                          Saved   |
|      Every [1] minute                                   |
|  [ ] Publish your changes                               |
|      After [5] minutes of inactivity                    |
|  [x] Watch folders for changes                          |
| Accounts                                                |
|  open.quiltdata.com   Role [analyst v]      [Sign out]  |
|  custom.registry.io   Signed out             [Sign in]  |
| Storage                                                 |
|  Where packages live  /home/user/QuiltSync      [Open]  |
|  App storage          [Check size]     [Free up space]  |
| App                                                     |
|  [ ] Keep running in the tray when the window is closed |
| Publishing                                              |
|  Message template  Generated from the changes           |
|  Default workflow  Each bucket's default                |
|  Default metadata  No metadata                          |
|                                              [Edit…]    |
| Troubleshooting                                         |
|  Report a problem              [Report a problem…]      |
|  Log level  [Default v]                                 |
|  Logs folder  ...  [Open]     Data folder  ...  [Open]  |
| Experimental                                            |
|  [ ] Agent tasks                                        |
| About                                                   |
|  Version  0.22.10                    [Release notes]    |
| Ignore rules                                            |
|  Your rules      ...                       [Open file]  |
|  Built-in rules                            [Review…]    |
+---------------------------------------------------------+
```

- Toggles, the log level and the minute fields save as they change, and
  the row says *Saved* for a moment. While a control's save is out it is
  held: a toggle or the log level greyed, a minute field read-only with the
  cursor still in it. Saves run in the order they were made;
  then the page reads again. A failed save says so in the band under the
  appbar, and the control goes back to what is stored.
- A minute field saves on Enter or leaving it, only a whole number of at
  least 1. The pull minutes are the interval while the window is focused;
  the unfocused and closed intervals stay as stored. A stored time that is
  not whole minutes shows rounded and is written only when changed.
- Publishing shows the stored defaults; **Edit…** opens the **Publishing
  Dialog**.
- Accounts are read on their own, so asking each host's role does not hold
  the page. **Sign in** returns here; **Sign out** asks first. A role pick
  switches at once.
- *Check size* measures `.quilt/` and what Free up space would free; the
  answer stays until you leave the page. **Free up space** works without a
  measure, and updates a measured line from what it freed.
- **Report a problem…** opens a dialog: an optional *What went wrong?*, the
  list of what is sent, **Send to Quilt** (collects the logs, sends them
  with your text) and **Email instead** (collects the logs, opens your mail
  app with the text and the zip's path).
- Turning on *Agent tasks* adds an *Agent tasks* card beside *Ignore rules*.

---

### Error

Generic error page with recovery options, at `/error`. The title, the
message, and **Reload page**, **Open .quilt directory**, **Login** and
**Go home**; **Login** shows only when there is a host. *Open .quilt
directory* reports in the band under the appbar. An unknown address draws the
same shape with *Page not found* and **Go home**.

### Opening a Quilt link

`/remote-package` runs the link as it opens. It shows a spinner, with
**Refresh** disabled, then replaces itself with the package page. A failure
draws the error buttons (Reload page, Open .quilt directory, Go home) in the
same frame; Reload page runs the link again without reloading the window. A
link that needs a sign-in or setup goes there instead, replacing this page so
Back does not run the link again.

---

## Flow Diagram

```text
                        +-------+
                        | Setup |  (first run only)
                        +---+---+
                            |
                            v
  +---------------------------------------------------+
  |                     Main Page                     |<---------+
  +---+---------------+-------------------+-----------+          |
      |               |                   |                      |
      | click pkg     | [Create package]  | [gear]               |
      | or queue row  v                   v                      |
      |          [dialog]           +----------+                 |
      |          -> list re-reads   | Settings |                 |
      |                             +----------+                 |
      v                                                          |
+-----+--------------+   [Publish] / [Get latest]                |
|  Installed Package |----> runs, page re-reads                  |
|                    |                                           |
|                    |   [Resolve] -> resolve mode               |
|                    |----> Share mine / Replace mine            |
|                    |                                           |
|                    |   [Review before publishing…]             |
|                    |   [⋯] Create new revision                 |
|                    |----> +--------+                           |
|                    |      | Commit |                           |
|                    |      +---+----+                           |
|                    |          | Publish / Save                 |
|                    |<---------+                                |
|                    |                                           |
|                    |   [⋯] Remove                              |
+--------------------+----> confirm ----------------------------+


  Any page ----[needs auth]----> Login ----[back]----> original page
```

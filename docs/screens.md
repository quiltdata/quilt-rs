# Screens & Flows

ASCII wireframes for Quilt Sync's UI pages and user journeys.

## Pages

### Setup

First-run screen. User picks the home directory for Quilt data.

```
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

```
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

```
+--[appbar]----------------------------------------------+
| [logo]                                 [refresh] [gear] |
+---------------------------------------------------------+
|                                                         |
|  +---------------------------------------------------+  |
|  | user/package-a                        [Pull] [>]  |  |
|  +---------------------------------------------------+  |
|  | user/package-b            [Push] [Commit] [Pull]  |  |
|  +---------------------------------------------------+  |
|  | org/dataset-c                         [Pull] [>]  |  |
|  +---------------------------------------------------+  |
|  | local/my-data                        [Commit] [>] |  |
|  +---------------------------------------------------+  |
|                                                         |
+---------------------------------------------------------+
```

Local-only packages (no remote `manifest_uri`) show Commit but
no Pull/Push buttons and no "Open Remote" action.

Empty state:

```
+---------------------------------------------------------+
|                                                         |
|   No packages installed                                 |
|   Install packages from the catalog using deep links.   |
|                                                         |
|   [how-to-deep-link illustration]                       |
|                                                         |
+---------------------------------------------------------+
```

- Click package row -> **Installed Package**
- Click [Pull] -> runs pull flow, reloads
- Click [Push] -> runs push flow -> **Installed Package**
- Click [Commit] -> **Commit**
- Click [gear] -> **Settings**

---

### Installed Package

Shows contents of a single installed package: file entries
with checkboxes, status indicator, and a toolbar.

```
+--[appbar]----------------------------------------------+
| [logo]  user/package-a                 [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [< Packages]              [Uninstall] [Push] [Commit]  |
+---------------------------------------------------------+
| [status: 3 files modified]                              |
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
| [Install Selected Paths]                                |
+---------------------------------------------------------+
```

- [< Packages] -> **Installed Packages List**
- [Commit] -> **Commit**
- [Push] -> runs push flow, reloads
- [Uninstall] -> runs uninstall flow -> **Installed Packages List**
- [Install Selected Paths] -> runs install_paths flow, reloads
- [Ignore] -> opens **Ignore Popup** (for junk-detected files)
- [Ignored] -> opens **Un-ignore Popup** (for `.quiltignore`-matched files)

For local-only packages the toolbar shows [Commit] but no [Push].

---

### Commit

Form for committing local changes to a package.
Two-column layout: form on the left, file list on the right.

```
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
| [Commit]                                                |
+---------------------------------------------------------+
```

After commit -> **Installed Package**

---

### Merge

Shown when local and remote versions have diverged.
Offers two resolution options.

```
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

```
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

```
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

### Settings

Application settings and diagnostics.

```
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

```
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

```
                        +-------+
                        | Setup |  (first run only)
                        +---+---+
                            |
                            v
  +---------------------------------------------------+
  |          Installed Packages List (Home)            |<---------+
  +---+------------------+-------------------+--------+           |
      |                  |                   |                    |
      | click pkg        | [Commit]          | [gear]            |
      v                  v                   v                    |
+-----+-------+    +----+----+        +-----+----+               |
|  Installed   |    |         |        |          |               |
|  Package     +--->| Commit  |        | Settings |               |
|              |    |         |        |          |               |
+--+-+--+------+    +----+----+        +----------+               |
   | |  |                |                                        |
   | |  | [Uninstall]    | submit                                 |
   | |  +----------------+--------->------------------------------+
   | |
   | | [Push] when diverged
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


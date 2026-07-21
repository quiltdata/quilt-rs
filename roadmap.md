# QuiltSync roadmap

## How we scope work — build vs. borrow

> Vision: see the QuiltSync README.

We build the one hard, unique part ourselves and borrow the rest from two
systems that already do it well.

- **Build (ours):** local sync and a real native-desktop app — keeping the
  files on your machine and the ones in the cloud in agreement.
- **Borrow from quilt3:** the proven Python tool defines how data is packaged,
  hashed, and validated; we match it exactly.
- **Borrow from the Web Catalog:** the web app defines the friendly experience
  (search, AI, previews, wording); we mirror it rather than invent our own.

## Now — in flight

- **Gentle pull** *(M)* — updating your local copy no longer wipes work you
  haven't sent up yet; it pulls in others' changes while keeping yours.
- **Layered crate split** *(L)* — reorganizing the code into clean layers, so
  the same logic can run in the browser and the storage format can be swapped
  later without a rewrite.

## Next

- **Role switcher** *(M)* — switch between the access roles you have (different
  projects, or read-only vs. read-write) from inside the app.
- **quilt-uri hardening** *(S)* — tighten how the app reads package links, so
  a bad one fails early with a clear message, not deep inside an operation.
- **Finish hiding the machinery** *(M)* — replace Git-flavored wording (commit,
  push, remote) with plain language a scientist reads without a gloss.
- **Autopush self-recovery** *(M)* — when a temporary error pauses automatic
  syncing, it should resume on its own instead of staying silently stuck.
- **Rework the installed-package page** *(M)* — make it unambiguous whether
  "all files" is the current state ("everything is selected") or an action
  ("select everything now"); today the two are conflated.
- **Default ignores** *(S)* — automatically skip junk system files (like macOS
  `.DS_Store`) so they never get synced into a package.
- **Simpler delivery & updates** *(M)* — serve downloads and auto-updates from
  GitHub Releases instead of the marketing site's file host; more reliable,
  with a grace period so existing installs keep updating.

## Later / exploring

- **Iceberg-table manifests** *(L — blocked on Catalog design)* — store a
  package's file list as an Iceberg table instead of a text file, for speed at
  large scale; needs the web side designed first.

# QuiltSync roadmap

## How we scope work — build vs. borrow

> Vision: see the [QuiltSync README](quilt-sync/README.md).

We build the one hard, unique part ourselves and borrow the rest from two
systems that already do it well.

- **Build (ours):** local sync and a real native-desktop app — keeping the
  files on your machine and the ones in the cloud in agreement.
- **Borrow from quilt3:** the proven Python tool defines how data is packaged,
  hashed, and validated; we match it exactly.
- **Borrow from the Web Catalog and top sync apps:** the web app sets the
  friendly Quilt experience (search, AI, previews, wording) and apps like
  Dropbox and Google Drive set the bar for effortless sync; we mirror both
  rather than invent our own.

## Now — in flight

- **Layered crate split** *(L)* — reorganizing the code into clean layers, so
  the same logic can run in the browser and the storage format can be swapped
  later without a rewrite.

## Next

- **Role switcher** *(M)* — switch between the access roles you have, from
  inside the app.
- **quilt-uri hardening** *(S)* — tighten how the app reads package links, so
  a bad one fails early with a clear message, not deep inside an operation.
- **Finish hiding the machinery** *(M)* — replace Git-flavored wording (commit,
  push, remote) with plain language a scientist reads without a gloss.
- **Autopush self-recovery** *(M)* — when a temporary error pauses automatic
  syncing, it should resume on its own instead of staying silently stuck.
- **Rework the installed-package page** *(M)* — "select all" conflates
  three things: a state display ("everything is downloaded" — silently
  false once teammates add files), a one-time *download all listed*
  action, and the expected-but-missing standing *keep everything
  downloaded* (new remote files auto-download). Separate all three.
- **Default ignores** *(S)* — automatically skip junk system files (like macOS
  `.DS_Store`) so they never get synced into a package.
- **Usage telemetry that can count users** *(S)* — events exist but carry no
  dimensions: add the catalog host to every event and an anonymous install
  ID, so "how many people use the pilot, and how" and per-customer usage
  dashboards become answerable.
- **Simpler delivery & updates** *(M)* — serve downloads and auto-updates from
  GitHub Releases instead of the marketing site's file host; more reliable,
  with a grace period so existing installs keep updating.
- **Rework the index page** *(M)* — give the package list more structure and a
  filter, so you can find a package quickly instead of scanning a flat list.
- **Quick test-environment bootstrap** — a way to bootstrap a test environment
  quickly for local manual testing, pre-populated with installed packages,
  without breaking your current in-use installation. It can likely be done with
  test harnessing alone, without any production code.
- **Extract S3 operations** — move the S3 operations into their own crate, or at
  least into a mostly-independent module, like `workflow` and `object_hash`.

## Recently shipped

- **Gentle pull** — updating your local copy no longer wipes work you haven't
  sent up yet; it pulls in others' changes while keeping yours.

## Later / exploring

- **Iceberg-table manifests** *(L — blocked on Catalog design)* — store a
  package's file list as an Iceberg table instead of a text file, for speed at
  large scale; needs the web side designed first.

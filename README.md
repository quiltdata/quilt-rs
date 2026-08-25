# quilt

**Version your data like code — on your own laptop.** `quilt` gives the files
your scripts and AI agents read and write real revisions: each one immutable
and content-addressed, pushable to S3 when you want it shared. No server and
no cloud credentials required to start.

[![CI](https://github.com/quiltdata/quilt-rs/actions/workflows/test-quilt-rs.yaml/badge.svg)](https://github.com/quiltdata/quilt-rs/actions/workflows/test-quilt-rs.yaml)
[![crates.io](https://img.shields.io/crates/v/quilt-cli.svg)](https://crates.io/crates/quilt-cli)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

There are two ways to use it: the `quilt` CLI, and [QuiltSync](quilt-sync/),
a desktop app for people who would rather not open a terminal. Both are front
ends over one shared Rust library, [`quilt-rs`](quilt-rs/), which you can
also build on directly.

<!-- TODO: animated terminal demo (light/dark SVG) of the loop below. -->

## Install

Fastest, via [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall)
(downloads a prebuilt binary, no compile):

```bash
cargo binstall quilt-cli
```

Or build from source (needs Rust 1.97+):

```bash
cargo install quilt-cli
```

Prebuilt binaries cover macOS (Apple Silicon and Intel) and Linux x86_64, and
are attached to every
[`quilt-cli/v*` release](https://github.com/quiltdata/quilt-rs/releases).
Windows builds from source only
([#844](https://github.com/quiltdata/quilt-rs/issues/844)).

## Quickstart

This example uses two directories: `--home` is where `quilt` keeps every
package's working copy (pass it once; it is remembered after that,
[#838](https://github.com/quiltdata/quilt-rs/issues/838)), and `-s` is an
existing directory to import files from. Commands log at INFO by default;
prefix them with `RUST_LOG=warn` to get exactly the output shown
([#837](https://github.com/quiltdata/quilt-rs/issues/837)).

```bash
mkdir -p ~/plate-exports
printf 'sample,od600\nA1,0.42\n' > ~/plate-exports/plate1.csv

quilt --home ~/QuiltSync create -n lab/assays -s ~/plate-exports -m "first run"
```

```text
Created package "lab/assays"
```

Your working copy now lives at `~/QuiltSync/lab/assays/`. Edit it like any
other directory, then ask what changed:

```bash
echo "A2,0.38" >> ~/QuiltSync/lab/assays/plate1.csv
quilt status -n lab/assays
```

```text
Local-only package (no remote origin)
+------------+----------+
| Changes:              |
+------------+----------+
| path       | status   |
+------------+----------+
| plate1.csv | Modified |
+------------+----------+
```

Commit the revision. Every commit is a content-addressed manifest — the hash
is the version:

```bash
quilt commit -n lab/assays -m "add well A2"
```

```text
New commit "efe0ddeca2fb6faf1455492f2ee673a883b961c9d1c0827c93680e7389bb8785" created
```

That is the whole local loop: `create`, edit, `status`, `commit`, `list`.

When you are ready to share, log in to your Quilt stack and push — `quilt
login` prompts for a one-time code, which you get from
`https://quilt.example.com/code`. Replace `quilt.example.com` with your own
stack's hostname (no stack yet?
[open.quiltdata.com](https://open.quiltdata.com) is a public demo you can log
in to and install from). The first push of a local-only package names the
destination bucket and host:

```bash
quilt login --host quilt.example.com
quilt push -n lab/assays --bucket lab-data --origin quilt.example.com
```

Collaborators log in to the same stack — on their machine, the first command
also sets `--home` once — then install. `install` by itself fetches only the
manifest, the content-hashed file listing; `--path` (or a `&path=` param in
the URI) downloads the files they actually need:

```bash
quilt --home ~/QuiltSync login --host quilt.example.com
quilt install --path plate1.csv \
  'quilt+s3://lab-data#package=lab/assays&catalog=quilt.example.com'
```

From then on, `quilt pull -n lab/assays` updates the files they installed to
each new revision. To inspect a remote package without installing anything,
`quilt browse <PKG_URI>` prints its manifest.

## Or skip the terminal: QuiltSync

[QuiltSync](quilt-sync/) is the same engine with a desktop UI, built for
people who would rather not use a CLI at all. Find a package in the Quilt
Catalog, open it on your desktop, and pull down only the files you need into
an ordinary local folder. Edit them in your usual tools; QuiltSync syncs the
changes back as a new immutable revision, and surfaces divergence when someone
else published in the meantime.

Builds for macOS (Apple Silicon and Intel), Windows, and Linux are on the
[releases page](https://github.com/quiltdata/quilt-rs/releases) under
`QuiltSync/v*`. The Windows installers are Authenticode-signed, and the macOS
app is signed and notarized. QuiltSync and the CLI operate on the same
packages and revisions, so you can move between them freely.

## Why this exists

We build AI learning loops for biotech, and the unit of agentic execution
turned out to be a **transaction across immutable, versioned artifacts** —
not a checkout of a repo. An agent that regenerates a dataset needs the
old revision to still exist, addressable by hash, with a record of what
changed. That is true on a laptop before it is ever true in a cloud account,
so `quilt` works fully local-first and reaches for S3 only when you need to
share or run out of disk.

**Why not git-lfs?** No server to run and no smudge/clean filters. Objects are
addressed by content hash in a store S3 understands natively, so `push`
uploads only objects whose content actually changed, and `install --path`
fetches part of a package without materializing the rest. That partial, dedup-first
model is what makes thousand-file, terabyte packages workable.

## What belongs in a Quilt package

Data: the structured and unstructured files that are the inputs, outputs, and
control surface of your pipelines and agents. Parquet, CSV, FASTQ, HDF5,
images, PDFs, notebooks, prompts, configs, model weights. These files are
often binary and sometimes enormous; what makes a revision valuable is that
it is immutable, addressable, and cheap to fetch in part.

Not your source tree. Code is text that gets reviewed, syntax-checked, built,
and run, and git is very good at that — keep it there. What changes for data
is the useful granularity: on a 40 GB Parquet file nobody wants a line-level
merge, but "which files changed, and which revision produced this result" is
exactly the right question.

`quilt status` answers the first half for your working copy, and the Quilt
web catalog compares any two revisions of a package; the CLI does not have a
`diff` verb yet.

The two compose: code in git, the data that code consumes and produces in
`quilt`, each referencing the other by hash.

## Current limitations

We chose to ship something simple that handles large binary files and
arbitrary document types over something complete. These are the sharp edges
that choice left, not positions we intend to defend forever. Each has an
issue, and [roadmap.md](roadmap.md) records how the work is sequenced.
Telling us which one actually blocks you is the most useful feedback you can
give — it is how we order the queue.

- **No undo yet.** No `revert`/`reset` verb, and a local-only package has no
  rollback path at all
  ([#840](https://github.com/quiltdata/quilt-rs/issues/840)).
- **No revision log yet** in the CLI
  ([#841](https://github.com/quiltdata/quilt-rs/issues/841)). QuiltSync and
  the Quilt catalog show a package's revision history in the meantime.
- **Noisy output.** Commands log at INFO on stdout; `RUST_LOG=warn` fixes it
  today ([#837](https://github.com/quiltdata/quilt-rs/issues/837)).
- **Disk usage only grows.** `objects/` and the manifest cache are never
  pruned. Content is shared across packages with no reference counting, so
  leaking bytes beats deleting something another package still addresses.
  Refcounted pruning is a real feature we have not built.
- **Divergence is resolved per package, not per file.** When two people move
  past the same base, you pick your manifest or theirs. Relatedly, a first
  push certifies itself as `latest` even if a teammate published that
  namespace first. Both are deliberate given binary payloads;
  [Resolving Diverged](docs/architecture.md#resolving-diverged) records the
  reasoning and the exact gaps versus git.
- **Prebuilt CLI binaries for macOS and Linux only.** Windows builds from
  source ([#844](https://github.com/quiltdata/quilt-rs/issues/844)), though
  QuiltSync ships a signed Windows app.

## What is in this repo

- **[`quilt-cli/`](quilt-cli/)** — the `quilt` binary
  ([crates.io](https://crates.io/crates/quilt-cli))
- **[`quilt-rs/`](quilt-rs/)** — the library it is built on
  ([crates.io](https://crates.io/crates/quilt-rs))
- **[`quilt-uri/`](quilt-uri/)** — WASM-safe Quilt+ URI parsing
  ([crates.io](https://crates.io/crates/quilt-uri))
- **[`quilt-sync/`](quilt-sync/)** — QuiltSync, the cross-platform desktop app

## Documentation

- [Architecture](docs/architecture.md) — design commitments, invariants, and
  operation contracts
- [Mental model](docs/mental-model.md) — the four-hash state model and the
  divergence classifier
- [Artifacts](docs/artifacts.md) — file and directory inventory, local and
  remote
- [Verification](docs/verification.md) — SHA256-chunked, CRC64/NVMe, and
  manifest hash recipes
- [Changelog](quilt-cli/CHANGELOG.md)

## Feedback wanted

We built this for our own pain and we are curious whether it maps to yours.
"This does not fit my workflow, here is why" is exactly what we want to
hear — open an
[issue](https://github.com/quiltdata/quilt-rs/issues/new/choose) and say so.
[CONTRIBUTING.md](CONTRIBUTING.md) covers dev setup, testing, and the release
process.

Apache-2.0 licensed.

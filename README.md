# quilt

**Version your data like code — on your own laptop.** `quilt` gives the files
your scripts and AI agents read and write real revisions: immutable,
content-addressed, visible in `status`, and pushable to S3 when you want them
shared. No server and no cloud credentials required to start.

[![CI](https://github.com/quiltdata/quilt-rs/actions/workflows/test-quilt-rs.yaml/badge.svg)](https://github.com/quiltdata/quilt-rs/actions/workflows/test-quilt-rs.yaml)
[![crates.io](https://img.shields.io/crates/v/quilt-cli.svg)](https://crates.io/crates/quilt-cli)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

<!-- TODO: animated terminal demo (light/dark SVG) of the loop below. -->

## Install

```bash
cargo binstall quilt-cli   # prebuilt binary, no compile
cargo install quilt-cli    # from source, needs Rust 1.97+
```

Prebuilt binaries cover macOS (Apple Silicon and Intel) and Linux x86_64, and
are attached to every
[`quilt-cli/v*` release](https://github.com/quiltdata/quilt-rs/releases).
Windows builds from source only
([#844](https://github.com/quiltdata/quilt-rs/issues/844)).

## Quickstart

Pick a directory where your packages will live. You only pass `--home` once —
it is remembered after that
([#838](https://github.com/quiltdata/quilt-rs/issues/838)).

```bash
mkdir -p ~/quilt-demo
printf 'region,q3\nwest,120\n' > ~/quilt-demo/sales.csv

quilt --home ~/QuiltSync create -n demo/sales -s ~/quilt-demo -m "initial import"
```

```text
Created package "demo/sales"
```

Your working copy now lives at `~/QuiltSync/demo/sales/`. Edit it like any
other directory, then ask what changed:

```bash
echo "east,98" >> ~/QuiltSync/demo/sales/sales.csv
quilt status -n demo/sales
```

```text
Local-only package (no remote origin)
+-----------+----------+
| Changes:             |
+-----------+----------+
| path      | status   |
+-----------+----------+
| sales.csv | Modified |
+-----------+----------+
```

Commit the revision. Every commit is a content-addressed manifest — the hash
is the version:

```bash
quilt commit -n demo/sales -m "add east region"
```

```text
New commit "efe0ddeca2fb6faf1455492f2ee673a883b961c9d1c0827c93680e7389bb8785" created
```

That is the whole local loop: `create`, edit, `status`, `commit`, `list`.
Commands are chatty by default right now; prefix with `RUST_LOG=warn` for the
output shown above ([#837](https://github.com/quiltdata/quilt-rs/issues/837)).

When you are ready to share, `quilt login --host <stack>`, then
`quilt push -n demo/sales --bucket <bucket> --origin <host>`. Collaborators
run `quilt install quilt+s3://<bucket>#package=demo/sales` and
`quilt pull -n demo/sales`.

## Why this exists

We build AI learning loops for biotech, and the unit of agentic execution
turned out not to be a workspace running a repo — it is a **transaction across
immutable versioned artifacts**. An agent that regenerates a dataset needs the
old revision to still exist, addressable by hash, with a record of what
changed. That is true on a laptop before it is ever true in a cloud account,
so `quilt` works fully local-first and grows into S3 only when you outgrow
your disk.

**Why not just git?** Git tracks line-level text history in a single working
copy. Data packages here are frequently binary and often enormous (Parquet,
FASTQ, HDF5 — thousands of files, terabytes), so `quilt` optimizes for partial
installs (`--path`), content-addressed dedup across packages, and
whole-manifest revisions rather than text diffs and merges.

**Why not git-lfs?** No server to run, no smudge/clean filters, and objects
are addressed by content hash in a store that S3 understands natively — `push`
uploads only rows whose content actually changed.

## When *not* to use this

- You want line-level diffs, merges, or blame on text files. Use git.
- You need to undo a commit. There is no `revert`/`reset` verb yet, and for a
  local-only package there is no rollback at all
  ([#840](https://github.com/quiltdata/quilt-rs/issues/840)).
- You want a revision log. Not in the CLI yet
  ([#841](https://github.com/quiltdata/quilt-rs/issues/841)) — use `browse`
  against a remote, or QuiltSync.
- You are tight on disk. `objects/` and the manifest cache are **never
  pruned**, deliberately: content is shared between packages and there is no
  reference counting, so the safe choice is to leak bytes rather than delete
  content another package still addresses.
- You expect merge semantics. Divergence is resolved by choosing a whole
  manifest — yours or theirs — not per file. The first push of a package also
  always certifies itself as `latest`, even if a teammate published that
  namespace first. Both are deliberate;
  [Resolving Diverged](docs/architecture.md#resolving-diverged) lists the gaps
  versus git explicitly.

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
"This does not fit my workflow, here is why" is the most useful thing you can
send us — open an
[issue](https://github.com/quiltdata/quilt-rs/issues/new/choose) and say so.
[CONTRIBUTING.md](CONTRIBUTING.md) covers dev setup, testing, and the release
process.

Apache-2.0 licensed.

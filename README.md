# quilt

**Version your data like code — on your own laptop.** `quilt` gives the files
your scripts and AI agents read and write real revisions: immutable,
content-addressed, visible in `status`, and pushable to S3 when you want them
shared. No server and no cloud credentials required to start.

This repo holds three ways to use it: the `quilt` CLI, the `quilt-rs` library
underneath it, and [QuiltSync](quilt-sync/), a desktop app for people who
would rather not open a terminal.

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

## Or skip the terminal: QuiltSync

[QuiltSync](quilt-sync/) is the same engine with a desktop UI, built for
people who would rather not use a CLI at all. Find a package in the Quilt
Catalog, open it on your desktop, and pull down only the files you need into
an ordinary local folder. Edit them in your usual tools; QuiltSync syncs the
changes back as a new immutable revision, and surfaces divergence when someone
else published in the meantime.

Builds for macOS (Apple Silicon and Intel), Windows, and Linux are on the
[releases page](https://github.com/quiltdata/quilt-rs/releases) under
`QuiltSync/v*`; the Windows artifacts are code-signed. Same packages, same
revisions, same registry as the CLI — use either, or both.

## Why this exists

We build AI learning loops for biotech, and the unit of agentic execution
turned out not to be a workspace running a repo — it is a **transaction across
immutable versioned artifacts**. An agent that regenerates a dataset needs the
old revision to still exist, addressable by hash, with a record of what
changed. That is true on a laptop before it is ever true in a cloud account,
so `quilt` works fully local-first and grows into S3 only when you outgrow
your disk.

**Why not git-lfs?** No server to run and no smudge/clean filters. Objects are
addressed by content hash in a store S3 understands natively, so `push`
uploads only rows whose content actually changed, and `install --path` fetches
part of a package without materializing the rest. That partial, dedup-first
model is what makes thousand-file, terabyte packages workable.

## What belongs in a Quilt package

Data: the structured and unstructured files that are the inputs, outputs, and
control surface of your pipelines and agents. Parquet, CSV, FASTQ, HDF5,
images, PDFs, notebooks, prompts, configs, model weights. Content that is
often binary, sometimes enormous, and valuable because a given revision is
immutable, addressable, and cheap to fetch in part.

Not your source tree. Code is text that gets reviewed, syntax-checked, built,
and run, and git is very good at that — keep it there. Line-level diff, blame,
and three-way merge are the right tools for a 200-line module and meaningless
on a 40 GB Parquet file, which is why `quilt` does not implement them.

The two compose: code in git, the data that code consumes and produces in
`quilt`, each referencing the other by hash.

## Current limitations

We chose to ship something simple that handles large binary files and
arbitrary document types over something complete. These are the sharp edges
that choice left, not positions we intend to defend forever. Each has an
issue — telling us which one actually blocks you is the most useful feedback
you can give, and it is how we order the work.

- **No undo yet.** No `revert`/`reset` verb, and a local-only package has no
  rollback path at all
  ([#840](https://github.com/quiltdata/quilt-rs/issues/840)).
- **No revision log yet** in the CLI
  ([#841](https://github.com/quiltdata/quilt-rs/issues/841)). Use `browse`
  against a remote, or QuiltSync.
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
"This does not fit my workflow, here is why" is the most useful thing you can
send us — open an
[issue](https://github.com/quiltdata/quilt-rs/issues/new/choose) and say so.
[CONTRIBUTING.md](CONTRIBUTING.md) covers dev setup, testing, and the release
process.

Apache-2.0 licensed.

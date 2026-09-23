# Contributing to Quilt Workspace

This repository contains multiple projects in a unified workspace:

- **[quilt-rs](quilt-rs/)** - Rust library for accessing Quilt data packages
  (built on [aws-sdk-rust](https://github.com/awslabs/aws-sdk-rust) and
  [Tokio](https://tokio.rs/))
- **[quilt-uri](quilt-uri/)** - Parser and types for Quilt+ URIs
- **[quilt-cli](quilt-cli/)** - Command-line interface for Quilt data packages
  (built with [clap](https://github.com/clap-rs/clap))
- **[quilt-sync](quilt-sync/)** - Cross-platform desktop GUI application built
  with [Tauri](https://tauri.app/) and a [Leptos](https://leptos.dev/) frontend
  compiled to WebAssembly (QuiltSync)

## Project-Specific Contributing Guides

For detailed contributing information, see the project-specific guides:

- **[quilt-rs Contributing Guide](quilt-rs/CONTRIBUTING.md)** - Releasing the
  crates.io crates (`quilt-uri`, `quilt-rs`, `quilt-cli`)
- **[QuiltSync Contributing Guide](quilt-sync/CONTRIBUTING.md)** - Desktop
  application development

## Development Workflows

This project uses `just` as a task runner for common development tasks.

```bash
cargo install just
cargo install cargo-nextest --locked   # the test runner CI uses
cargo install rumdl                    # the markdown linter CI uses

just -l
```

`just lint` reports, and writes nothing: `cargo fmt --check` over the workspace,
`rumdl check` over the tracked markdown, then clippy for the host and wasm
targets. CI checks all three, so a clean `just lint` leaves nothing for it to
reject. `just fmt` is the half that writes, over the same file set.

QuiltSync's frontend is a Rust-to-WebAssembly crate built by Trunk, so `just
gallery` (the component gallery in a browser) and `just start` (the desktop app)
need the frontend toolchain as well:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk stylance-cli       # the page bundler and the CSS-module compiler
# Node.js and npm, for the JSON editor bundle Trunk's pre-build hook installs
```

`just start` additionally needs `cargo install tauri-cli` and Tauri's platform
dependencies, listed at <https://tauri.app/start/prerequisites/>.

The two can run at the same time. They build into separate directories, `ui/dist`
for the app and `ui/dist-gallery` for the gallery, because Trunk writes every
target it is given to `<dist>/index.html` and a shared directory would leave
whichever rebuilt last owning the page both of them serve.

A bare cargo command covers the workspace's default members (`quilt-rs`,
`quilt-cli`, `quilt-sync`), not `quilt-uri` or `quilt-sync-ui`. Add
`--workspace` for every member, as `just test` does, or `-p` for one:

```bash
# Testing
cargo test --workspace              # All workspace packages
cargo test -p quilt-rs              # Specific package only

# Building, formatting, linting follow the same pattern
cargo build [--workspace | -p package-name]
cargo fmt --all [--check]           # or -p package-name
cargo clippy [--workspace | -p package-name]
```

### Tests that need AWS

Tests that read or write the shared S3 fixtures are named `live_*`. Nothing
marks them as skipped, so `cargo test` and `cargo nextest run` both run them by
default and both need AWS credentials in the environment.

Without credentials, deselect them by name:

```bash
just test-no-aws            # the recipe; wraps the line below
cargo nextest run --profile no-aws
cargo test -- --skip live_  # same effect without nextest
```

Note that `cargo test --profile no-aws` does **not** work — to `cargo`,
`--profile` names a build profile, and it will fail with `profile 'no-aws' is
not defined`. The `no-aws` profile belongs to nextest.

CI splits the same line: one step runs the `no-aws` selection everywhere, and a
second step runs `live_*` with credentials, skipped when the pull request comes
from a fork. GitHub withholds secrets from fork pull requests — a platform
rule, not a project choice — so the split is what lets an outside contributor
get a CI signal at all.

**The naming convention is load-bearing.** A test that touches the fixtures but
is not named `live_*` lands in the credential-free step, and fails there on
every run — including your own pushes, not just fork pull requests. If a test
you just wrote fails with a credentials error, check its name first.

## Filing an issue

An issue is something someone can pick up and finish, which needs two parts:
what cannot be done today, observable from outside, and how anyone will know
when it is done. A direction with no acceptance criteria is not a bad issue —
it is not an issue, and belongs on the
[Roadmap](https://github.com/quiltdata/quilt-rs/issues/889) instead.

State a specific about the code — a file, a field, what the fix involves —
only where you checked it. An unchecked guess in the problem statement reads
as a premise and gets built to; the same guess in the form's unverified box
costs nobody anything.

## Getting a pull request merged

Three things are checked on every pull request. Meeting them is necessary
rather than sufficient — a change still has to be correct, readable, and free
of anything that weakens security, and review will say so where it is not. But
a pull request missing any of these will be sent back, so they are the cheap
ones to check first.

**CI is green.** Which checks run depends on what you touched: the Rust
workflows ignore markdown-only changes, `test-quilt-rs` also ignores
`quilt-sync/**`, and the markdown lint fires only on `*.md`. QuiltSync's
Cross-Platform Tests job is `main`-only and never appears on a pull request at
all. So read the check list for what is there, not for the absence of red — a
check that never ran is not a check that passed. A fork gets the same set as a
branch here, minus the `live_*` tests, which are skipped rather than failed
(see "Tests that need AWS" above).

**Every review comment is resolved, and @greptileai is at 5/5.** Reviews come
from Greptile and Copilot as well as from maintainers. The bots are usually
right and sometimes wrong — a reply explaining why a comment does not apply
resolves it just as well as a code change does. Silence does not. If Greptile
holds below 5/5 over something we have deliberately decided against, say so in a
reply and a maintainer will merge past it.

**The docs still tell the truth.** This is the one most pull requests miss. The
`README.md` files describe current behaviour, and what is missing is tracked in
the [Roadmap](https://github.com/quiltdata/quilt-rs/issues/889), so a change
that closes a gap tends to falsify a sentence somewhere else. Grep for the
issue number *and* for the behaviour you changed; they do not find the same
lines. A command that gains a flag, a default, or a subcommand usually touches
the root `README.md` quickstart, the crate's own `README.md`, and the Roadmap
line that named the gap — a roadmap entry is a claim about the product too, and
it goes stale the same way a README paragraph does.

Changelog entries are welcome but not required — a maintainer will write one
if you have not.

## Release Process Overview

Each project has different release approaches:

- **quilt-uri**, **quilt-rs**: Libraries published to crates.io, plus a
  GitHub Release, by `release-crate.yaml`
- **quilt-cli**: Published to crates.io by `release-crate.yaml`, which also
  attaches prebuilt binaries for macOS (x86_64, aarch64) and Linux
  (x86_64-gnu) to its GitHub Release; install via `cargo binstall quilt-cli`
  or `cargo install quilt-cli`
- **QuiltSync**: Cross-platform installers on a GitHub Release by
  `release-quilt-sync.yaml`, mirrored to HubSpot (where the auto-updater
  looks) by `upload-to-hubspot.yaml`

All three workflows are run manually, and every release waits as a draft for
a maintainer's approval.

### Version Management

Every released crate has its own version in its own `Cargo.toml` and its own
`CHANGELOG.md`; there is no shared workspace version. QuiltSync's version is
in `quilt-sync/src-tauri/Cargo.toml`; `quilt-sync-ui` is not released.
`quilt-rs` and `quilt-cli` name the upstream crates' versions in their
path-dependency `version =` specifiers, so the crates are released as a
cascade, `quilt-uri` → `quilt-rs` → `quilt-cli`.

### Unreleased Versions

Between releases, a crate with unreleased changes carries a `-dev`
version in `Cargo.toml` (e.g., `0.39.2-dev`), and its `CHANGELOG.md`
opens with a matching `## [v0.39.2-dev]` heading and no date, not
`[Unreleased]`. The first PR to change a crate after its release opens
the cycle: it sets the next patch `-dev` version and adds the heading
with its entry. The release PR drops `-dev` and adds the date. If your
change needs a bigger bump than the cycle has (a feature in a patch
cycle), rename the version in both files. When `quilt-uri` or
`quilt-rs` goes `-dev`, move the downstream `version =` specifiers for
it to the same `-dev` version: Cargo's version requirements skip
pre-releases. That alone does not open a cycle for the downstream crate.

The unreleased section describes the change since the last release, not
a history of PRs. If your PR makes an earlier unreleased entry stale —
supersedes, extends, or reverts it — rewrite that entry in place and
append your PR link to it rather than adding a new one. Released
sections are never edited. Work no user can reach yet, not even through
Settings → Experimental, gets a single "Under the hood" line rather than
an Added or Changed entry; it earns a real entry once it becomes
reachable, even behind an Experimental switch.

See [docs/releases.md](docs/releases.md) for the release steps, and the
project-specific contributing guides for what differs per crate.

## File Integrity Verification

See [docs/verification.md](docs/verification.md) for SHA256-chunked,
CRC64/NVMe, and manifest verification recipes.

## Reporting Security Issues

See [SECURITY.md](SECURITY.md) for how to report vulnerabilities privately
and what is in scope.

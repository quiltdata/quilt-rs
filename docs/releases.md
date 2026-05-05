<!-- markdownlint-disable MD013 -->
# Releases

What ships, where it goes, and which workflow puts it there. For why
the workspace is split this way, see
[architecture.md → Workspace Crate Layout](architecture.md).

## Workspace layout

One Cargo workspace, six `Cargo.toml` files (root + five members).
QuiltSync contributes two of the five members.

| `Cargo.toml` | Crate | Version | Released to |
| --- | --- | --- | --- |
| root | — | — | (workspace manifest) |
| `quilt-rs/` | `quilt-rs` | 0.31.0 | crates.io + GitHub Release |
| `quilt-uri/` | `quilt-uri` | 0.2.0 | crates.io + GitHub Release |
| `quilt-cli/` | `quilt-cli` | 0.25.2 | crates.io + GitHub Release with prebuilt binaries |
| `quilt-sync/src-tauri/` | `quilt-sync` | 0.17.1 | GitHub Release + HubSpot mirror |
| `quilt-sync/ui/` | `quilt-sync-ui` | 0.1.0 | **not released** — built into QuiltSync |

Path-dep version specifiers are kept in sync manually:

- `quilt-rs/Cargo.toml`: `quilt-uri = { path = "...", version = "..." }`.
- `quilt-cli/Cargo.toml`: `quilt-rs = { ... }` and `quilt-uri = { ... }`.
- `quilt-sync/*` use path-only deps (no `version =`), so internal bumps
  don't touch them.

Each released crate owns a `CHANGELOG.md`; see the header comment in
each one for the conventions (alpha pre-release pattern, autolink
format, cross-crate subsections).

## Release workflows

All are `workflow_dispatch` only — nothing fires on tag push, merge,
or schedule.

| Workflow | Inputs | Output |
| --- | --- | --- |
| `release-crate.yaml` | `crate` ∈ {`quilt-rs`, `quilt-uri`, `quilt-cli`} | crates.io publish + GitHub draft release |
| `release-quilt-sync.yaml` | `environment` (GitHub Environment) | GitHub draft release with cross-platform installers; gated `promote` job flips draft → public + dispatches `upload-to-hubspot.yaml` |
| `upload-to-hubspot.yaml` | `release_tag`, `hubspot_folder` (default `/quiltsync/`) | Mirrors a published QuiltSync release to HubSpot Files + writes a HubDB row. Triggered automatically by the QuiltSync `promote` job; can also be re-run manually if HubSpot upload flakes. |

Every release in this repo is created with `--draft` and
`--latest=false`. `make_latest=false` is **sticky** — promoting a
draft via `gh release edit <tag> --draft=false` does not flip it back.
No workflow ever sets `make_latest=true`; the only release marked as
"latest" is the QuiltSync one, and it gets that flag manually during
the review-and-publish step (see [Manual steps](#manual-steps-the-part-thats-not-in-any-workflow)).

## Crates.io publishing — `release-crate.yaml`

Three jobs in sequence: `prepare` → `binaries` (quilt-cli only) →
`publish`. `publish` is gated by the `crates-io` GitHub Environment
(required reviewers configured in repo settings), so nothing reaches
crates.io until a maintainer approves the draft.

`prepare` job (`ubuntu-24.04`, no environment):

1. Read the crate's version from `cargo metadata` (Cargo.toml is the
   source of truth).
2. Slice release notes from `<crate>/CHANGELOG.md` via
   `parse-changelog`.
3. `gh release create <crate>/v<version> --draft --latest=false`.
4. Print the draft URL to the workflow summary so the reviewer has a
   one-click link.

`publish` job (`environment: crates-io`, required reviewers):

1. `gh release edit <tag> --draft=false` (`--latest=false` is sticky
   from creation, so this stays opted out of "latest").
2. Mint a short-lived crates.io token via
   `rust-lang/crates-io-auth-action` (OIDC trusted publishing — no
   `CARGO_REGISTRY_TOKEN` secret).
3. `cargo publish -p <crate>`.

GitHub promote runs first so that if `cargo publish` flakes, the
recoverable side (the GitHub Release) has already shipped and only the
crates.io step needs a re-run; the reverse order is irrecoverable.

`binaries` job — runs only when `crate == 'quilt-cli'`:

| target | runner |
| --- | --- |
| `x86_64-apple-darwin` | `macos-15` |
| `aarch64-apple-darwin` | `macos-15` |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |

For each, `cargo build -p quilt-cli --release --target <target>`,
package as `quilt-cli-<target>.tar.gz` (binary inside a directory
named `quilt-cli-<target>/`), upload to the draft release. `binaries`
is a hard dependency for `publish`, so the gate cannot fire until
every archive is attached — the reviewer can download and run
`quilt --version` against the actual artifact before approving.

The asset filename **must** mirror `quilt-cli/Cargo.toml`'s
`[package.metadata.binstall]`:

```toml
pkg-url = "{ repo }/releases/download/{ name }/v{ version }/{ name }-{ target }.tar.gz"
pkg-fmt = "tgz"
bin-dir = "{ name }-{ target }/{ bin }{ binary-ext }"
```

Drift breaks `cargo binstall quilt-cli`.

End-user install paths produced by these releases:

- `cargo install quilt-rs` / `cargo install quilt-uri` (libraries).
- `cargo binstall quilt-cli` → prebuilt archive from the GitHub Release.
- `cargo install quilt-cli` → falls back to building from source.

## QuiltSync — `release-quilt-sync.yaml`

Single `release` job, parallel matrix:

| Platform | Runner | Tauri args |
| --- | --- | --- |
| macOS arm64 | `macos-15` | `--target aarch64-apple-darwin` |
| macOS x86_64 | `macos-15` | `--target x86_64-apple-darwin` |
| Linux x86_64 | `ubuntu-24.04` | (none) |
| Windows x86_64 | `windows-latest` | (none) |

Build pipeline:

1. Setup Rust (release cache, with `wasm32-unknown-unknown` everywhere
   for the Leptos UI; macOS additionally adds both Apple targets).
2. Setup Node 24 + Trunk (for `quilt-sync/ui`).
3. `tauri-apps/tauri-action@v0` runs `trunk build --release` on the UI
   (via `beforeBuildCommand`), then bundles the targets listed in
   `tauri.conf.json` (`["app", "deb", "dmg", "msi"]`). With
   `createUpdaterArtifacts: true` it also produces `latest.json`,
   `*.app.tar.gz`, and `*.msi.zip` updater bundles. Tag pattern:
   `QuiltSync/v__VERSION__` (Tauri substitutes `__VERSION__` from
   `quilt-sync/src-tauri/Cargo.toml`) — this is the final tag form,
   matching the input `upload-to-hubspot.yaml` expects. Release is
   created as a draft.
4. (Windows only) `azure/login@v3` exchanges the GitHub OIDC token for
   an Azure access token, then `azure/trusted-signing-action@v1` signs
   every `.exe` and `.msi`. See
   [docs/windows-signing.md](windows-signing.md).

Linux uses `mold` linker via
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-C link-arg=-fuse-ld=mold`
to avoid OOM on memory-constrained runners.

Secrets / variables consumed:

- macOS notarization: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`.
- Telemetry: `MIXPANEL_API_SECRET`, `MIXPANEL_PROJECT_TOKEN`, `SENTRY_DSN`.
- Updater signing: `TAURI_SIGNING_PRIVATE_KEY` (+ password). Public key
  is embedded in `tauri.conf.json`.
- Azure (Windows signing): `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
  `AZURE_SUBSCRIPTION_ID`, `AZURE_TRUSTED_SIGNING_*`.

## Auto-updater (QuiltSync only)

`quilt-sync/src-tauri/tauri.conf.json` configures HubFS as the sole
endpoint:

```jsonc
"endpoints": [
  "https://www.quilt.bio/hubfs/latest.json"
]
```

This makes `upload-to-hubspot.yaml` the actual rollout switch — until
HubFS is mirrored, no client picks up the new version. Promoting the
GitHub release is no longer enough on its own, which is the property
that lets "GH public, HubSpot pending" stay invisible to users.

## HubSpot mirror — `upload-to-hubspot.yaml`

Run **after** a QuiltSync GitHub release has been promoted. Inputs:

- `release_tag` — e.g. `QuiltSync/v1.2.3`. The QuiltSync workflow
  emits this exact form on the draft, so the input matches without
  any manual rename.
- `hubspot_folder` — base path on HubFS, default `/quiltsync/`.

Steps:

1. `gh release download "$RELEASE_TAG"` into `./release-assets/`.
2. Rewrite every `url` in `latest.json` to point at
   `https://www.quilt.bio/hubfs<hubspot_folder><release_tag>/<basename>`.
3. POST every asset to the HubSpot Files API:
   - `latest.json` → root, so its public URL stays
     `https://www.quilt.bio/hubfs/latest.json` regardless of version
     (this is what the auto-updater hits).
   - everything else → `<hubspot_folder><release_tag>/`.
4. Insert a HubDB row with version, release tag, `latest.json`
   content, deduped installer URL list, and per-platform URLs (Windows
   MSI, macOS arm/intel DMG, Linux DEB) derived by globbing the
   release assets. Then publish the HubDB draft so the row appears on
   the quilt.bio download page.

Required: secret `HUBSPOT_ACCESS_TOKEN`, repo vars `HUBDB_TABLE_ID`
and `HUBDB_COLUMN_MAP`.

## Manual steps (the part that's not in any workflow)

For any release:

- Bump `version` in the relevant `Cargo.toml` and update path-dep
  specifiers. The released crates are dependent on each other and
  must be released as a cascade: `quilt-uri` → `quilt-rs` → `quilt-cli`.
  Bumping an upstream crate requires bumping every downstream crate
  that depends on it (and updating that downstream `Cargo.toml`'s
  `version =` specifier for the path dep).
- Replace the latest `*-alphaN` block in the matching `CHANGELOG.md`
  with a real version + today's date.
- Run the workflow (`workflow_dispatch`).
- Review the draft (for `quilt-cli`, download the archives and confirm
  `quilt --version`; for QuiltSync, install one of the bundles), then
  approve the gated `publish`/`promote` job in the Actions UI. The
  approval is the only manual step:
  - **Crate releases** — `publish` promotes the draft and runs
    `cargo publish`.
  - **QuiltSync** — `promote` flips the draft to public, marks it as
    "latest", and dispatches `upload-to-hubspot.yaml` (HubFS + HubDB
    mirror) in one step.

  No `gh release edit` and no separate Actions-tab click are needed.

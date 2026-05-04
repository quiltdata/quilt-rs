# quilt-cli

Command-line interface for [Quilt](https://docs.quilt.bio/) data packages.
Thin wrapper around [`quilt-rs`](../quilt-rs/) — see
[`docs/architecture.md`](../docs/architecture.md) for what each command does
under the hood.

The binary is named `quilt`.

## Install

Recommended (downloads a prebuilt binary):

```sh
cargo binstall quilt-cli
```

Prebuilt binaries are currently published for macOS (x86_64, aarch64)
and Linux (x86_64-gnu). On other platforms, or if `cargo-binstall` is
not installed, build from source:

```sh
cargo install quilt-cli
```

## Commands

| Command     | Purpose                                          |
| ----------- | ------------------------------------------------ |
| `browse`    | Fetch and inspect a remote manifest              |
| `create`    | Create a new local-only package                  |
| `install`   | Install a remote package locally                 |
| `status`    | Show working-directory changes                   |
| `commit`    | Commit a new package revision                    |
| `push`      | Upload a local revision to the remote            |
| `pull`      | Fetch the latest remote revision                 |
| `list`      | List installed packages                          |
| `uninstall` | Remove a package from local tracking             |
| `login`     | Authenticate against a Quilt stack               |

Run `quilt <command> --help` for arguments.

## Global flags

- `--domain <path>` — local domain directory (stores credentials and
  package metadata). Defaults to
  `~/.local/share/com.quiltdata.quilt-rs/`.
- `--home <path>` — directory where packages keep their working files.
  Required on the first invocation against a domain (every command
  validates that a home is set); afterward it is persisted in the
  domain lineage and may be omitted.

## Example

```sh
quilt --home ~/QuiltHome login --host open.quiltdata.com
quilt install quilt+s3://quilt-example#package=akarve/cord19 \
    --namespace akarve/cord19
quilt status --namespace akarve/cord19
```

`--home` is needed once to initialize the domain; the later commands pick
it up from the saved lineage.

URIs follow the [Quilt+ URI format](https://docs.quilt.bio/quilt-platform-catalog-user/uri).

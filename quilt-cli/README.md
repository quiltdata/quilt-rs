# quilt-cli

**Version your data like code — on your own laptop.** `quilt` gives the files
your scripts and AI agents read and write real revisions: immutable,
content-addressed, visible in `status`, and pushable to S3 when you want them
shared. No server and no cloud credentials required to start.

Thin wrapper around [`quilt-rs`](../quilt-rs/) — see
[`docs/architecture.md`](../docs/architecture.md) for what each command does
under the hood, and the
[repository README](https://github.com/quiltdata/quilt-rs#readme) for
positioning and known limitations.

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
| `undo-commit` | Undo the newest commit, before the first push  |
| `push`      | Upload a local revision to the remote            |
| `pull`      | Fetch the latest remote revision                 |
| `list`      | List installed packages and their commit status  |
| `log`       | List the revisions this copy has, newest first    |
| `uninstall` | Remove a package from local tracking             |
| `login`     | Authenticate against a Quilt stack               |
| `role`      | Show or switch your active role on a stack       |

`list`'s status compares commits: your last commit against the last-known
remote tip, read from local records so listing stays offline. It is not the
package's overall state — uncommitted edits are invisible to it, so a package
with local changes still shows `up_to_date`. `quilt status` reads the working
copy.

`install` fetches the package manifest and starts tracking it; files are
downloaded only for the paths you name with `--path` (repeatable) or a
`&path=` parameter in the URI.

Run `quilt <command> --help` for arguments.

## Global flags

- `--domain <path>` — local domain directory (stores credentials and
  package metadata). Defaults to the platform local-data directory under
  `com.quiltdata.quilt-sync/`, shared with QuiltSync
  (`~/.local/share/com.quiltdata.quilt-sync/` on Linux,
  `~/Library/Application Support/com.quiltdata.quilt-sync/` on macOS).
- `--home <path>` — directory where packages keep their working files.
  Defaults to `~/QuiltSync` on first use and is persisted in the domain
  lineage. Pass `--home` only to store packages somewhere else.

`list` and `status` accept `--json` for a machine-readable form, so
`quilt list --json | jq` works. Field names are stable; the human tables are
not, so parse the JSON rather than the tables.

Commands keep stdout reserved for command output by default. Add `-v` or
`--verbose` to show INFO-level logs on stderr; set `RUST_LOG` for target-specific
filtering.

## Example

```sh
quilt login --host open.quiltdata.com
quilt install \
    "quilt+s3://quilt-example#package=akarve/cord19&catalog=open.quiltdata.com"
quilt status --namespace akarve/cord19
```

Package files are stored under `~/QuiltSync` by default. Pass `--home` only
when you want a different package directory. The namespace defaults to the one
in the URI, so `install` needs no `--namespace` here.

URIs follow the [Quilt+ URI format](https://docs.quilt.bio/quilt-platform-catalog-user/uri).
With `&catalog=<host>`, S3 requests use the stack credentials from
`quilt login`; without it, the CLI uses your local AWS credentials from
`~/.aws`.

## Fully local workflow

No login or remote is required to create, edit, and version a package
entirely on disk:

```sh
quilt create --namespace me/local-pkg --message "Initial revision"

# Package files live under ~/QuiltSync/<namespace> by default; add/edit them directly
cd ~/QuiltSync/me/local-pkg
echo "a,b,c" > data.csv

quilt status
quilt commit --message "Add data.csv"

quilt list
```

Add `--source <dir>` to `create` to populate the package from an existing
directory instead of starting empty. The package stays local-only — usable
with `status`, `commit`, and `uninstall` — until you `push` a revision to a
remote (the first push requires `--bucket` and `--origin`).

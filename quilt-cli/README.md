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

Prebuilt binaries are currently published for macOS (x86_64, aarch64),
Linux (x86_64-gnu), and Windows (x86_64-msvc). On other platforms, or if
`cargo-binstall` is not installed, build from source:

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
| `role`      | Show or switch your active role on a stack       |

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
  Required on the first invocation against a domain (every command
  validates that a home is set); afterward it is persisted in the
  domain lineage and may be omitted. See
  [#838](https://github.com/quiltdata/quilt-rs/issues/838).

Commands log at `INFO` on stdout by default; `RUST_LOG=warn` leaves only
command output. See
[#837](https://github.com/quiltdata/quilt-rs/issues/837).

## Example

```sh
quilt --home ~/QuiltHome login --host open.quiltdata.com
quilt install \
    "quilt+s3://quilt-example#package=akarve/cord19&catalog=open.quiltdata.com"
quilt status --namespace akarve/cord19
```

`--home` is needed once to initialize the domain; the later commands pick
it up from the saved lineage. The namespace defaults to the one in the
URI, so `install` needs no `--namespace` here.

URIs follow the [Quilt+ URI format](https://docs.quilt.bio/quilt-platform-catalog-user/uri).
With `&catalog=<host>`, S3 requests use the stack credentials from
`quilt login`; without it, the CLI uses your local AWS credentials from
`~/.aws`.

## Fully local workflow

No login or remote is required to create, edit, and version a package
entirely on disk:

```sh
# --home is only needed on the first invocation against a domain
quilt --home ~/QuiltHome create --namespace me/local-pkg \
    --message "Initial revision"

# Package files live under <home>/<namespace>; add/edit them directly
mkdir -p ~/QuiltHome/me/local-pkg
echo "a,b,c" > ~/QuiltHome/me/local-pkg/data.csv

quilt status --namespace me/local-pkg
quilt commit --namespace me/local-pkg --message "Add data.csv"

quilt list
```

Add `--source <dir>` to `create` to populate the package from an existing
directory instead of starting empty. The package stays local-only — usable
with `status`, `commit`, and `uninstall` — until you `push` a revision to a
remote (the first push requires `--bucket` and `--origin`).

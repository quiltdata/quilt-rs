# quilt-cli

**Version your data like code — on your own laptop.** `quilt` gives the files
your scripts and AI agents read and write real revisions: immutable,
content-addressed, visible in `status`, and pushable to S3 when you want them
shared. No server and no cloud credentials required to start.

Thin wrapper around [`quilt-rs`](../quilt-rs/) — see
[`docs/architecture.md`](../docs/architecture.md) for what each command does
under the hood, and the
[repository README](https://github.com/quiltdata/quilt-rs#readme) for
positioning, and the
[Roadmap](https://github.com/quiltdata/quilt-rs/issues/889) for what is missing
and roughly when.

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
| `home`      | Print or set the folder packages keep files in   |
| `install`   | Install a remote package locally                 |
| `status`    | Show working-directory changes                   |
| `commit`    | Commit a new package revision                    |
| `undo-commit` | Undo the newest commit, before the first push  |
| `push`      | Upload a local revision to the remote            |
| `pull`      | Fetch the latest remote revision                 |
| `list`      | List installed packages and their commit status  |
| `log`       | List the revisions this copy has, and why each is kept |
| `uninstall` | Remove a package; `--prune` also deletes its files |
| `gc`        | Delete stored files no installed package uses    |
| `remove-revisions` | Remove old revisions this copy no longer needs |
| `login`     | Authenticate against a Quilt stack               |
| `role`      | Show or switch your active role on a stack       |

`list`'s `last synced` column compares commits: your last commit against the
remote tip as of the package's last install, pull or push, read from local
records so listing stays offline. It is not the package's overall state —
uncommitted edits are invisible to it, so a package with local changes still
shows `synced`. `quilt status` reads the working copy and checks the remote now.

`quilt list --fetch` asks each package's remote now and counts its changed
files, giving the answer QuiltSync's main page gives, in its words: `latest`,
`not the latest`, `3 files changed`, `revision not published`, `changed in both
places`, `not published yet`, `no S3 bucket yet`, `no access`, `unknown`. It
leaves local packages unchanged, so a plain `list` afterwards still shows the
last sync. When a remote cannot be reached, that row falls back to what was
last recorded and says so, as in `latest (remote unreachable)`. Under `--json`
each row adds `fetched`, `true` only when the remote was read just now, a
`changed_files` count when the files were scanned, and `"access_denied": true`
when the remote refused your role; `status` keeps its values.

`install` fetches the package manifest and starts tracking it; files are
downloaded only for the paths you name with `--path` (repeatable) or a
`&path=` parameter in the URI.

`pull` reports the files it moved, in four groups: downloaded, left on the
remote, updated, removed. A path you do not track is absent from all four — a
pull writes nothing for it, so a copy that installed no paths reports no files
at all. The message that follows is the newest revision's only: a pull advances
to `latest` in one step and may span several revisions.

`home` prints the home, the folder where installed packages keep their files.
It defaults to `~/QuiltSync` on first use. `quilt home <dir>` sets it; a
relative `<dir>` is taken against the current directory, and a missing folder
is created. While any package is installed, a different home is refused,
because the packages' files stay in the old folder. `--overwrite` changes it
anyway, and the installed packages then point at empty folders, so their files
read as deleted. `quilt home --json` prints `{"home": "<path>"}`.

`log` lists the revisions this copy holds, newest first. Each says why it is
kept, or how much removing it would free, so it shows what `remove-revisions`
would remove:

```text
revision  obtained (UTC)       kept                  frees     message
a1b2c3d4  2026-10-01 09:12:00  current · not pushed            fix labels
e5f6a7b8  2026-09-28 14:03:11  latest · base                   v3
0c9d8e7f  2026-09-20 10:40:52                        210.4 kB  v2
```

Telling an unpublished revision apart asks the remote's registry. If that
fails, `log` still lists the revisions, leaves out `kept` and `frees`, and warns
on stderr. Under `--json` each revision has `kept`, a list of reasons (empty
when removable), and `frees`, in bytes or `null` when kept; both are `null`
when the registry could not be read.

`remove-revisions` removes the old revisions of a package this copy holds,
oldest first, with the stored files only they use, and prints what it freed:
`Removed 4 old revisions of user/plate-07 · freed 630.2 kB`. `--count N`
removes only the N oldest. It keeps the revision the working files are at, the
remote's latest, the merge base, a commit not yet pushed, and any revision the
registry does not list, since this copy is then the only place it is. With
nothing to remove it says so and succeeds. Under `--json` it prints the
`removed` hashes, oldest first, with `revisions`, `objects` and `bytes` freed;
`kept_for` names a package that was busy, so the files were kept for `quilt
gc`.

Run `quilt <command> --help` for arguments.

## Global flags

`--domain`, `--format`, `--json` and `--verbose` are global: each goes before
or after the command, so `quilt --domain <path> list` and
`quilt list --domain <path>` both work.

- `--domain <path>` (`-d`) — local domain directory (stores credentials and
  package metadata). Defaults to the platform local-data directory under
  `com.quiltdata.quilt-sync/`, shared with QuiltSync
  (`~/.local/share/com.quiltdata.quilt-sync/` on Linux,
  `~/Library/Application Support/com.quiltdata.quilt-sync/` on macOS).
  Without the flag, `quilt` reads the domain from `QUILT_DOMAIN`, so
  `export QUILT_DOMAIN=<path>` points every command in a shell at one domain.
  The flag wins over the variable, and an empty `QUILT_DOMAIN` counts as unset.
- `--format text|json` — format of the command's result: human-readable
  `text` (the default) or machine-readable `json`. Every command accepts it.
  Without the flag, `quilt` reads the format from `QUILT_FORMAT`, so
  `export QUILT_FORMAT=json` makes every command in a shell print JSON. The
  flag wins over the variable, and an empty `QUILT_FORMAT` counts as unset.
  Logs and warnings on stderr are never affected.
- `--json` — shorthand for `--format json`, so `quilt list --json | jq` and
  `quilt --json list | jq` both work. It also wins over `QUILT_FORMAT`, but
  cannot be combined with `--format`.
- `--verbose` (`-v`) — show quilt's INFO-level logs on stderr. It wins over
  `QUILT_LOG`. Commands keep stdout reserved for command output.

### Logs

Logs go to stderr; by default only warnings and errors. Set `QUILT_LOG` to see
more:

- A level — `trace`, `debug`, `info`, `warn`, `error` or `off`. `QUILT_LOG=debug`
  shows quilt's own debug lines and keeps its dependencies (the HTTP stack, the
  AWS SDK) at warnings; `error` and `off` apply to everything.
- [`tracing` directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives),
  used as given, for example `QUILT_LOG=quilt_rs=trace,aws_smithy_runtime=debug`.

An empty `QUILT_LOG` counts as unset, and `-v` wins over it. A value that is
neither, such as `QUILT_LOG=debgu`, stops the command before it runs, even with
`-v`. `--format` never changes logs.

`--home <path>` is deprecated, use `quilt home <path>`. It goes before the
command, prints a warning, then sets the home the same way before the command
runs.

`quilt push --origin <host>` (`-o`) is deprecated, use `quilt push --host
<host>`, the name `login` and `role` already use. It prints a warning, then
pushes the same way.

### JSON output

Field names are stable; the human tables are not, so parse the JSON rather than
the tables.

On any failure the command itself reports (argument errors come from the parser
and stay human-readable, exit 2), the payload goes to stderr as
`{"error": {"kind": "...", "message": "..."}}`, stdout stays empty, and the exit
status is unchanged. `kind` is a stable identifier to branch on; `message` is the
same prose the human form prints.

stderr is also the log stream, so the error object is not necessarily the whole
of it: a warning at the default level, or INFO lines under `-v`, are written
there first. The object is always a single line and always the last one, so
read it with `tail -n 1` rather than parsing the stream whole.

## Example

```sh
quilt login --host open.quiltdata.com
quilt install \
    "quilt+s3://quilt-example#package=akarve/cord19&catalog=open.quiltdata.com"
quilt status --namespace akarve/cord19
```

Package files are stored under `~/QuiltSync` by default. Run
`quilt home <dir>` first when you want a different package directory. The
namespace defaults to the one in the URI, so `install` needs no `--namespace`
here.

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
remote (the first push requires `--bucket` and `--host`).

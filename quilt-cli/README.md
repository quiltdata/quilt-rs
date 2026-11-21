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
| `home`      | Print or set the folder packages keep files in   |
| `install`   | Install a remote package locally                 |
| `keep`      | Show or set which of a package's files are kept  |
| `status`    | Show working-directory changes                   |
| `commit`    | Commit a new package revision                    |
| `undo-commit` | Undo the newest commit, before the first push  |
| `push`      | Upload a local revision to the remote            |
| `pull`      | Fetch the latest remote revision                 |
| `read`      | Read a dataspace's `read` entries, with no Domain but its local registry |
| `publish`   | Publish a dataspace's `write` and `replace` entries, with no Domain but its local registry |
| `reattach`  | List or reattach local commits a dataspace's lock lost |
| `reset`     | Discard a dataspace package's local commits and changes for `latest` |
| `certify`   | Make a dataspace package's current revision its `latest` |
| `list`      | List installed packages and their commit status  |
| `log`       | List the revisions this copy has, and why each is kept |
| `uninstall` | Remove a package; `--prune` also deletes its stored files no other package uses |
| `gc`        | Delete stored files no installed package uses    |
| `storage`   | Print how much the dataspace Domain holds         |
| `local-registry` | Print or rename this machine's local registry |
| `remove-revisions` | Remove old revisions this copy no longer needs |
| `catalog`   | Show, set or clear the default Quilt stack       |
| `login`     | Authenticate against a Quilt stack               |
| `role`      | Show or switch your active role on a stack       |
| `version`   | Print the version, as `quilt --version` does     |

`list`'s `last synced` column compares commits: your last commit against the
remote tip as of the package's last install, pull or push, read from local
records so listing stays offline. It is not the package's overall state —
uncommitted edits are invisible to it, so a package with local changes still
shows `synced`. `quilt status` reads the working copy and checks the remote now.

`quilt list --fetch` asks each package's remote now and counts its changed
files, giving the answer QuiltSync's main page gives, in its words: `latest`,
`not the latest`, `3 files changed`, `revision not published`, `changed in both
places`, `not published yet`, `no S3 bucket yet`, `folder missing`, `no access`,
`unknown`. It
leaves local packages unchanged, so a plain `list` afterwards still shows the
last sync. When a remote cannot be reached, that row falls back to what was
last recorded and says so, as in `latest (remote unreachable)`. Under `--json`
each row adds `fetched`, `true` only when the remote was read just now, a
`changed_files` count when the files were scanned (never for a missing
folder), and `"access_denied": true` when the remote refused your role;
`status` keeps its values.

`status` ends with how many files and folders the ignore rules leave out, as
in `3 ignored files and 1 ignored folder not included`. `--ignored` lists each
one, a folder as one row, with the rule that ignored it and where the rule
comes from: `built-in`, your rules file in the domain folder with its line, or
the package's `.quiltignore` with its line:

```text
+-----------+-------+----------------+
| Ignored:                           |
+-----------+-------+----------------+
| path      | rule  | source         |
+-----------+-------+----------------+
| .git/     | .*/   | built-in       |
+-----------+-------+----------------+
| notes.tmp | *.tmp | .quiltignore:3 |
+-----------+-------+----------------+
1 ignored file and 1 ignored folder not included
```

Under `--json`, `ignored` holds the `files` and `folders` counts, and with
`--ignored` a `paths` list whose entries carry `path`, `kind` (`file` or
`folder`), `rule`, `source` (`built-in`, `user` or `package`), and `line`,
`rules_file` and `size` where they apply.

A package URI's `@hash` may be a prefix of 6 to 63 lowercase hex characters,
as in quilt3: `install` and `browse` look it up in the bucket's registry, and
fail when no revision starts with it or several do, naming the candidates.

`install` fetches the package manifest and starts tracking it; files are
downloaded only for the paths you name with `--path` (repeatable) or a
`&path=` parameter in the URI. A path the package already tracks is left as
it is and listed as `Already installed: "<path>"` (`already_installed` under
`--json`); the other paths install.
Unless `--no-download` is given, `--keep-all` or `--keep-folder` also downloads
what the package then keeps, as `keep` does.

`keep` sets which of a package's files this copy keeps: `--files`, the files
you picked (the default); `--all`, every file in the package; or
`--folder <dir>` (repeatable), every file under those folders. A folder is a
path from the package root, need not exist yet, and covers only the paths under
it, so `data/raw` does not cover `data/raw2`; `--folder ./` is `--all`. It then
downloads the files the setting covers that are not here yet; `--no-download`
only saves the setting and says how many files wait. With no flag it prints the
setting. Every `pull` follows it, so files a new revision adds under a kept
folder, or anywhere under `--all`, are downloaded too. Under `--json` it prints
`namespace`, `scope` (`individual_files`, `entire_package` or `folders`),
`folders` for a folders setting, and `outstanding`, the files still waiting.

`pull` reports the files it moved, in four groups: downloaded, left on the
remote, updated, removed. A file the new revision adds that the package's
setting does not keep is listed as left on the remote. A change to or removal
of a file you do not track and the setting does not keep is not listed: the pull
writes nothing for it. The message that follows is the newest revision's only:
a pull advances to `latest` in one step and may span several revisions.
When the package is already at `latest`, `pull` prints `Already up to date` and
exits 0; under `--json` the same shape has `"hash": null` and empty groups.

`read <DATASPACE>` reads the folder's `data.toml` and puts each package it
lists under `read` in its working folder: the revision's files under the
entry's paths, and all of the revision that is not a file as `metadata.toml`:
its message, workflow and metadata, with `"$NULL"` for JSON `null`. A revision
holding an integer beyond TOML's 64 bits is refused. It uses no Domain but its
local registry, where `bio.quilt.dataspace` exists in the OS's local data
folder: a `quilt+local://` package is read from it, and left unread without
one. The domain folder supplies only a catalog's login. A file changed
here is kept, unless the revision changed it too: then that package is blocked,
nothing of it changes, and the colliding files are listed and written to the
dataspace's `conflict.json`, under the package, with what the revision
(`incoming`) and the folder (`current`) hold. Set a file's `resolved` there to
`""` to delete it, or to the `file://` URL or versioned `s3://` URL of the file
to put there, and the next `read` takes it while the row still describes both
sides; a file so deleted is filled again by a later `read`, as any missing file
is. Each package read
whole has its revision recorded in `data.lock.toml`, and `latest` is never
moved. A working folder that is missing or empty is filled at the revision the
lock names, unless `data.toml` names one or that revision can no longer be read
whole, and the next `read` moves it on. A
lock from another host, user or folder is used only while every working folder
is empty and it names no unpushed commits; otherwise remove it. A revision
some of whose files are gone from the bucket, or do not match their hash,
changes nothing. Packages under `write`, `replace` and `sync` are left alone.
It exits 1 when any package was not read whole. Under `--json` it prints
`dataspace`, `completed` and `entries`, each with `package`, `folder`,
`outcome` (`read`, `refilled`, `incomplete`, `blocked`, `unpushed` or
`failed`),
`completed`, `revision`, the `placed`, `removed`, `kept`, `resolved`,
`skipped`, `hash_mismatch` and `conflicts` lists, and `error`.

`publish <DATASPACE>` publishes each package the folder's `data.toml` lists
under `write` or `replace` from its working folder, with no Domain but its
local registry, which a `quilt+local://` package is published to: `write`
lays the files under the entry's paths over `latest`, and `replace` publishes
the folder as the whole package, with `metadata.toml` setting the message and
metadata. What a commit leaves out is left out: files the folder's
`.quiltignore`, the domain folder's user-wide one or the built-in rules ignore.
A package also listed under `read` publishes only on its base, the revision the
lock names; where `latest` has moved past it, the package is read first, as
`read` would, and a read that does not complete publishes nothing. Each
revision that becomes `latest` is recorded in `data.lock.toml`; a lock from
another host, user or folder is refused. It exits 1 unless `latest` holds
every package's folder. Under `--json` it prints `dataspace`, `completed`
and `entries`, each with `package`, `folder`, `access`, `outcome`
(`published`, `stored`, `unchanged`, `behind`, `not_read`, `unpushed` or
`failed`), `completed`, `revision`, `latest`, `latest_unchecked`, `read` (the
read run first, as `read` reports an entry, or `null`) and `error`.

`read` and `publish` run the scripts `data.toml` declares under
`[hooks.pre-read]`, `[hooks.post-read]`, `[hooks.pre-write]` and
`[hooks.post-write]`, each a `command` with its `args` and `env`, from the
dataspace's folder, with `QUILT_HOOK`, `QUILT_DATASPACE`, `QUILT_PACKAGE`,
`QUILT_FOLDER` and, after a read or write, `QUILT_REVISION` set. Their output
goes to standard error. A `pre-` script that fails leaves its package unread
or unpublished, with the failure as its `error`; `post-read` runs only for a
package read at a newer revision, and `post-write` for each revision stored.

`reattach <DATASPACE> <PACKAGE>` lists the package's local commits that
`data.lock.toml` does not name, as after the lock was deleted: those its bucket
lacks, kept in `bio.quilt.dataspace` in the OS's local data folder. Commits
made with `quilt commit` without `--dataspace` are not among them.
`reattach ... <COMMIT> --base <HASH>` attaches one as the package's unpushed
commit on that base, which its next publish expects `latest` to be.
`reattach ... <COMMIT> --unpublished` attaches a commit of a package never
published, with no base: its next publish expects no `latest`. PACKAGE is
`quilt+s3://bucket#package=owner/name`, listed under `write`, `replace` or
`sync`. A lock that already names unpushed commits of the package, or that
another host, user or folder wrote, is refused. Under `--json` it prints
`package`, `unattached` (`hash` and `message`; `null` after an attach),
`attached` and `base` (`null` with `--unpublished`).

`install <PKG_URI> --dataspace <DIR>` adds the package to the folder's
`data.toml`, making the file if there is none, under `--access` (`read`,
`write`, `replace` or `sync`, the default), then reads it into its working
folder as `read` would and records the revision in `data.lock.toml`, after
`data.toml`. The entry is never pinned: a revision the URI names is read and
recorded in the lock, and the next read or sync moves the folder on. The
bucket is written only when it is not the contract's default `registry`, so the
folder is `owner/name/`, or `bucket/owner/name/` for another bucket, unless
`--folder` names one. `--path`, or a `&path=` in the URI, installs that part;
a later install of another path joins the same entry and folder, and the new
path is read at the lock's revision first. Installing what the lock already
names into a folder that holds it writes nothing. A package listed under
another access, in another folder or pinned to another revision is refused;
edit `data.toml` to change it. `quilt+local://#package=owner/name` installs from
the Domain's local registry. A `write` or `replace` entry is not read. It exits
1 when the package was not read whole. Under `--json` it prints `package`,
`access`, `folder`, `declared` (whether `data.toml` changed) and the read as
`read` reports an entry, its `outcome` also `unchanged` or `not_read`.

`uninstall --dataspace <DIR> <PACKAGE>`, or `-n owner/name` for the package
`data.toml` lists under that name, removes every entry of the package from
`data.toml`, deletes its working folder when a read filled it inside the
dataspace, and then drops it from `data.lock.toml`. A folder outside the
dataspace, or of a `write` or `replace` entry, is left. Local commits the lock
named that were not pushed stay in the Domain and are listed: install the
package again and `reattach` one. `--prune` then clears what the Domain holds
of the package that its bucket holds too, never its unpushed commits or the
local registry, and nothing while the bucket cannot be asked. Under `--json`
it prints `package`, `removed`, `kept`, `unpushed` and `pruned`.

`status --dataspace <DIR>`, `list --dataspace <DIR>` and
`log --dataspace <DIR> <PACKAGE>` read a dataspace and write nothing, not even
the Domain's caches. `status` shows each entry of `data.toml`: its base and
current revision from `data.lock.toml`, the registry's `latest`, asked now, and
whether it is up to date, behind, ahead or diverged; its working folder's
changes as a commit would take them, under the dataspace's `[ignore]` table;
its keys in `conflict.json`; a revision autosync stored without making it
`latest`; and its local commits not pushed. It also says whether the dataspace
was filled and what in the lock's header is not this machine's. `list` shows
each entry's state in a table, against the `latest` last known unless
`--fetch` asks each registry now. `log` lists the package's revisions in its
registry, newest first by when each was published, and the local commits not
pushed yet, marked `(local, not pushed)`. Under `--json`, `status` prints
`locked`, `filled`, `mismatches`, `problems` and `entries`, each with
`package`, `access`, `folder`, `state`, `upstream_state`, `base`, `current`,
`latest`, `remote`, `latest_source`, `unpushed`, `changes`, `conflicts`,
`kept` and `problems`; `log` prints `revisions`, each with `hash`, `message`,
`time`, `parent` and `local`.

`commit -m <MESSAGE> --dataspace <DIR> <PACKAGE>`, or `-n owner/name`,
commits the working folder of a `sync` entry as a local commit, as a sync
would: what the ignore rules leave out is left out, a file missing from the
folder is deleted, and `metadata.toml` sets the metadata, `-m` the message.
The commit is kept in `bio.quilt.dataspace` in the OS's local data folder, or
in its local registry for a `quilt+local://` package, and appended to the
package's commits in `data.lock.toml`; the next sync pushes the last one. The
`pre-write` hook runs first. `--meta` and `--workflow` do not apply: the
commit takes the bucket's default workflow, checked when it is pushed. A
`read`, `write` or `replace` entry is refused, and so is a package whose
folder is missing or empty while the lock names a revision, or one with a
revision after `data.toml` changed, until a sync reads it again. Under
`--json` it prints `package`, `commit` (`null` with nothing to commit),
`base` and `commits`.

`undo-commit --dataspace <DIR> <PACKAGE>` drops the package's last local
commit from `data.lock.toml` and restores the working folder to the revision
before it, the commit before or else the base, or nothing for a package never
published, as `undo-commit` restores a package in the home: files the commit
changed or deleted take back their bytes, files it added are removed, and
`metadata.toml` is rewritten. It refuses unless every file it would touch
holds what the commit put there, so nothing not committed is overwritten. The
commit stays in the Domain, where `reattach` lists it. A pushed commit is no
longer in the lock, so undo never steps back past a push. Under `--json` it
prints `package`, `undone`, `current`, `commits`, `restored` and `removed`.

`reset --dataspace <DIR> <PACKAGE>` discards the package's local commits and
local changes, reads the revision `latest` names whole into its folder, and
records it as the base in `data.lock.toml`. Each file of the package's
revisions is replaced or removed; a file neither holds was never committed and
is left. Every file is fetched before the folder changes, and a reset that
fails part-way puts the folder back. The discarded commits stay in the
Domain, for `reattach`. A `read` or `sync` entry is reset; a pinned entry and
a package with no `latest` are refused. Under `--json` it prints `package`,
`revision`, `discarded`, `placed` and `removed`.

`certify --dataspace <DIR> <PACKAGE>` makes the package's current revision,
its last local commit or else its base, the revision `latest` names: a commit
is pushed first, then `latest` is written whatever it names, over any other
publisher's, as QuiltSync's Certify latest does. A revision the bucket's
workflow refused is certified all the same, which releases it, and so is a
move from the local registry to a bucket the workflow refused. Only a `sync`
entry is certified. Under `--json` it prints `package`, `revision`,
`previous`, `pushed` and `workflow_refused`.

`remove-revisions --dataspace <DIR> <PACKAGE> --revision <HASH>` removes each
named revision, a top hash or a prefix of only one, from the package's
registry, as quilt3's `delete_package` with a registry does: a bucket's tags
naming it, or the local registry's manifest of it, never the data. Where
`latest` names one, it moves to the newest revision left, or is removed with
none left. No revision is protected, and `data.lock.toml` is left as it is:
the output says when it names a removed revision, which `reset` reads past.
Under `--json` it prints `package`, `removed`, `latest` (`kept`, `moved` or
`removed`) and `locked`.

`keep --dataspace <DIR> <PACKAGE>`, or `-n owner/name`, changes what the
dataspace keeps of the package, in `data.toml`: `--all`, `--folder <DIR>` and
`--path <PATH>` set the paths its entry covers, `--pin <REVISION>` and
`--unpin` its pin, which only a `read` entry takes, and `--access` moves it to
another list; `--from` names the list whose entry changes where the package is
in two. The package keeps its place in the file. Files a `read` or `sync` entry
newly covers are read at the revision `data.lock.toml` names before anything
can count them deleted, and only those: a file deleted under the old paths
stays deleted. Files it no longer covers stay in the folder, and a pin is read
by the next `read`. With no change it prints the entry. It exits 1 when the
newly covered files were not read; then `data.toml` holds the change and the
lock is left. Under `--json` it prints `package`, `access`, `paths` (`null` for
the whole package), `revision`, `folder`, `declared`, `outcome` and `placed`.

`create --dataspace <DIR> <PACKAGE>` declares a new package,
`quilt+s3://bucket#package=owner/name` or `quilt+local://#package=owner/name`,
in the dataspace's `data.toml` with no revision, under `--access` (`sync`, the
default, `write` or `replace`), and commits what its folder holds as its first
commit, with no base; the next sync publishes it, expecting no `latest`.
`--source <DIR>` names an existing folder to package where it is, inside the
dataspace or not; nothing is copied, and a folder holding the Domain is
refused. The metadata is the folder's `metadata.toml`; `-m` is the commit's
message. A package the dataspace, the lock or its registry already has is
refused, and so is an owner or name with characters other than letters,
digits, `_` and `-`. Under `--json` it prints `package`, `access`, `folder` and
`commit`.

`home --dataspace [<DIR>]` prints the default dataspace, which replaces the
home, or chooses `<DIR>`, making it a dataspace with an empty `data.toml` where
it has none. Nothing moves: the dataspace chosen before keeps its contract and
folders. The choice is kept in `default_dataspace.json` beside this
application's other settings, in the folder `--domain` names.

`gc --dataspace` clears the dataspace Domain, `bio.quilt.dataspace`: in each
bucket's store, the manifests the bucket holds and the objects only they use.
Commits not pushed and their objects are kept, the local registry is never
touched, and a store whose bucket cannot be asked, as offline, or that another
quilt process is changing, is left as it is and named; then it exits 1. Each
bucket is asked through the default catalog, or this machine's AWS credentials
where none is set. `--dry-run` prints what it would free and deletes nothing.
Under `--json` it prints `dry_run`, `manifests`, `objects`, `bytes` and
`stores`, by bucket.

`storage` prints how many bytes the dataspace Domain holds, each bucket's
store's and the local registry's, and reads no bucket. Under `--json` it prints
`total_bytes`, `registries` and `local_bytes`.

`local-registry` prints the name this machine's local registry keeps, and the
names it had; `local-registry <NAME>` renames it. The old name stays an alias,
so a `quilt+local://` URI, `data.toml` or `data.lock.toml` written with it
still names the registry, and the autosync work queued under it moves to the
new name. Under `--json` it prints `name`, `aliases` and `moved`.

`push` with no commit to push prints `Nothing to push` and exits 0; under
`--json` the same shape has `"hash": null`, `"manifest_uri": null` and
`"certified_latest": null`.

`commit` with no file changes and the same message and metadata would make the
same revision again, so it records nothing: it prints `Nothing to commit` and
exits 0; under `--json` it prints `{"hash": null}`. A new message or new
metadata is a new revision.

`commit` of a package with a remote checks the revision against the bucket's
current workflows, and a revision they would refuse is still committed: each
reason is printed as a `Warning:` line after the commit line, and under
`--json` it is a `warnings` list beside `hash`. `push` refuses that revision
until a new commit satisfies them. A package with no remote gets no check.

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

`version` prints what `quilt --version` prints. For a bug report,
`quilt version --details` adds the quilt-rs and quilt-uri versions, the OS and
architecture, and the folders quilt uses: the domain, whether it came from
`--domain`, `QUILT_DOMAIN` or the default, the default domain, and the home, or
`not set`. It only reads: it creates no folder and skips the newer-release
check. Under `--json` the same fields are `version`, `quilt_rs`, `quilt_uri`,
`os`, `arch`, `domain`, `domain_from`, `default_domain` and `home` (`null`
when not set).

Run `quilt <command> --help` for arguments.

## Global flags

`--domain`, `--format`, `--json`, `--log` and `--verbose` are global: each goes
before or after the command, so `quilt --domain <path> list` and
`quilt list --domain <path>` both work.

- `--domain <path>` (`-d`) — local domain directory (stores credentials and
  package metadata). Defaults to the platform local-data directory under
  `com.quiltdata.quilt-sync/`, shared with QuiltSync
  (`~/.local/share/com.quiltdata.quilt-sync/` on Linux,
  `~/Library/Application Support/com.quiltdata.quilt-sync/` on macOS).
  Without the flag, `quilt` reads the domain from `QUILT_DOMAIN`, so
  `export QUILT_DOMAIN=<path>` points every command in a shell at one domain.
  The flag wins over the variable, and an empty `QUILT_DOMAIN` counts as unset.
  QuiltSync does not read `QUILT_DOMAIN`, so a domain set this way is not
  the one the app uses.
- `--format text|json` — format of the command's result: human-readable
  `text` (the default) or machine-readable `json`. Every command accepts it.
  Without the flag, `quilt` reads the format from `QUILT_FORMAT`, so
  `export QUILT_FORMAT=json` makes every command in a shell print JSON. The
  flag wins over the variable, and an empty `QUILT_FORMAT` counts as unset.
  Logs and warnings on stderr are never affected.
- `--json` — shorthand for `--format json`, so `quilt list --json | jq` and
  `quilt --json list | jq` both work. It also wins over `QUILT_FORMAT`, but
  cannot be combined with `--format`.
- `--log <filter>` — which logs to show on stderr; takes the same values as
  `QUILT_LOG` (see [Logs](#logs)) and wins over it, so
  `quilt list --log debug` shows quilt's debug lines for one command.
  Commands keep stdout reserved for command output.
- `--verbose` (`-v`) — shorthand for `--log info`. It cannot be combined with
  `--log`.

### Logs

Logs go to stderr; by default only warnings and errors. Set `QUILT_LOG` or
pass `--log` to see more:

- A level — `trace`, `debug`, `info`, `warn`, `error` or `off`. `QUILT_LOG=debug`
  shows quilt's own debug lines and keeps its dependencies (the HTTP stack, the
  AWS SDK) at warnings; `error` and `off` apply to everything.
- [`tracing` directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives),
  used as given, for example `QUILT_LOG=quilt_rs=trace,aws_smithy_runtime=debug`.

`--log` or `-v` wins over `QUILT_LOG` and replaces it. An empty `QUILT_LOG`
counts as unset; an empty `--log` is an error. A value that is neither, such as
`QUILT_LOG=debgu` or `--log debgu`, stops the command before it runs, even when
a flag overrides the variable. `--format` never changes logs.

`--home <path>` is deprecated, use `quilt home <path>`. It goes before the
command, prints a warning, then sets the home the same way before the command
runs.

`quilt push --origin <host>` (`-o`) is deprecated, use `quilt push --host
<host>`, the name `login` and `role` already use. It prints a warning, then
pushes the same way.

### Newer releases

When a newer `quilt-cli` is published, `quilt` prints one yellow line on
stderr below the command's output, for example
`quilt 0.41.0 is available (you have 0.40.2). Run "quilt upgrade"`.
It asks crates.io at most every six hours and remembers the answer in your
cache folder; a check that fails or takes over two seconds is skipped without
a word. `NO_COLOR` drops the colour. Nothing is printed when stderr is not a
terminal or `CI` is set, and `QUILT_NO_UPDATE_CHECK=1` turns it off.

`quilt upgrade` installs the latest prebuilt release with cargo-binstall.
When no prebuilt binary can be installed, because cargo-binstall is missing or
failed, it asks before building from source with
`cargo install --locked quilt-cli`, which takes a few minutes. Unless both
stdin and stderr are a terminal, it asks nothing, builds nothing and names that
command. Without cargo it
points at the releases page, and on Windows it prints the command to run
instead. Afterwards
it prints the version of the `quilt` on your PATH, so you can see whether that
copy is the one cargo upgraded.

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

`quilt catalog set open.quiltdata.com` makes a stack the default, so `login`
and `role` can leave out `--host`; `quilt catalog` shows it and
`quilt catalog clear` forgets it. With none set, `quilt catalog` takes the
host of `navigator_url` from quilt3's `config.yml`, once, and keeps it in
`settings.json` in the shared folder (`QUILT_AUTH_DIR`). The default is never
used for a package, which keeps the catalog it came from, or none.

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
# Or with package metadata (a JSON object):
# quilt commit -m "Add data.csv" --meta '{"owner":"me"}'

quilt list
```

`commit` and `create` take `--meta '<json>'` (quilt3's name; `-u` and
`--user-meta` are the same flag). The value must be a JSON object; bad
JSON or a non-object (`--meta null`, an array, a string) is an error.
No flag on `commit` keeps the previous revision's metadata, including
the exact text of integers beyond 64 bits. No flag on `create` writes
no `user_meta` key. There is no flag for removing the key (`Clear` in
the library), which quilt3 does not have, and `push` has no `--meta`:
it publishes an existing commit.

Add `--source <dir>` to `create` to populate the package from an existing
directory instead of starting empty. The package stays local-only — usable
with `status`, `commit`, and `uninstall` — until you `push` a revision to a
remote (the first push requires `--bucket`, and `--host` when the bucket has
a Quilt stack; without one, the push uses the AWS credentials on this
computer).

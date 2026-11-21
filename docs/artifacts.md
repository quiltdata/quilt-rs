# Artifacts

## App data directory (`$DATA_DIR`)

*QuiltSync only.* Path provided by the OS via Tauri's `app_local_data_dir()`
for the bundle identifier `com.quiltdata.quilt-sync`. Not configurable.
CLI and library callers choose their own root directory.

| Platform | Path |
|----------|------|
| Linux    | `~/.local/share/com.quiltdata.quilt-sync/` |
| macOS    | `~/Library/Application Support/com.quiltdata.quilt-sync/` |
| Windows  | `%LOCALAPPDATA%\com.quiltdata.quilt-sync\` |

**Debug → Open .quilt dir** opens `$DATA_DIR/.quilt/` in the file manager.

## Home directory

Where package files (working copies) live. Each installed package occupies
`<owner>/<package>/` inside it.

*QuiltSync*: set by the user on the Setup page on first launch, then stored in
`$DATA_DIR/.quilt/domain.json` and read from there on subsequent starts.
Defaults to `~/QuiltSync` if the user has not configured it yet.
*CLI/library*: the caller provides the path.

## Domain (`$DATA_DIR/.quilt/domain.json`, `$DATA_DIR/.quilt/refs/`)

*All components (quilt-rs, quilt-cli, QuiltSync).* `domain.json` holds the
home directory path; `refs/<owner>/<package>.json` holds one installed
package's remote origin and local commit state. Path is hardcoded relative to
`$DATA_DIR`.

| Artifact | Path |
|----------|------|
| Package ref | `.quilt/refs/<owner>/<package>.json` |
| Remote tip cache, which may be missing or stale | `.quilt/refs/<owner>/<package>.json.latest` |
| Domain config | `.quilt/domain.json` |
| Package lock | `.quilt/locks/<owner>/<package>.lock` |
| Domain lock | `.quilt/domain.lock` |
| Move lock, held by a move to refs or back | `.quilt/migrate.lock` |

A domain kept in `data.json` by an earlier version moves here on its first
use, which keeps that file as `data.json.pre-refs` and leaves in `data.json` a
stub that earlier versions fail to read. Under `data.json.lock`, which
earlier versions write `data.json` under, the move copies `data.json` to
`data.json.migrating` and puts the stub in its place; it then reads only that
copy. `quilt lineage rollback` moves
the domain back: it writes `data.json.rollback` from the refs, moves `refs/`
to `refs.rolled-back-<uuid>/`, and puts that file in place of the stub. A
move or rollback a kill interrupted resumes from what these files show.

## Package state (`$DATA_DIR/.quilt/`)

*All components.* Managed by quilt-rs; CLI and QuiltSync use it through the
library.

| Artifact | Path | Path source |
|----------|------|-------------|
| Cached remote manifests | `$DATA_DIR/.quilt/packages/<bucket>/<hash>` | Bucket and hash from the remote manifest URI |
| Installed manifests | `$DATA_DIR/.quilt/installed/<ns>/<hash>` | Namespace from the package, hash from the manifest |
| Object store | `$DATA_DIR/.quilt/objects/<sha256hex>` | Hash derived from file content |

## Dataspace Domain (`bio.quilt.dataspace/`)

*quilt-rs.* The Domain that dataspaces use, beside `$DATA_DIR`, not in it.
`quilt_rs::domain::locate` places it: a `--domain` value, else
`QUILT_DOMAIN`, else `bio.quilt.dataspace/` in the OS's local data folder
(`~/Library/Application Support/` on macOS, `~/.local/share/` on Linux,
`%LOCALAPPDATA%` on Windows). No client selects it that way yet: QuiltSync
and quilt-cli still name `$DATA_DIR` with `--domain` and `QUILT_DOMAIN`.

| Artifact | Path |
|----------|------|
| Default dataspace, dataspaces on autosync, import state (format 2) | `domain.json` |
| Autosync's queue | `queue.db` |
| Rebuildable state | `state.db` |
| Staged writes | `staging/` |
| Domain lock, held by a writer of `domain.json` and by an import | `domain.lock` |
| A bucket's Package Store | `registries/<bucket>/{latest,packages,installed,objects,locks}/` |
| The local registry | `registries/@local/` |

`quilt_rs::domain::import::import` fills it from a domain on refs in
`$DATA_DIR/.quilt/`, which it only reads: each package's installed manifests
and the objects they name go to its bucket's store, or the local registry's
for a package with no remote, and each ref becomes a `sync` entry in the old
home's `data.toml` and `data.lock.toml`. The home becomes the default
dataspace. Going back is using `$DATA_DIR` again; nothing done in the Domain
since is carried back. An import that stopped part way runs again to the
end; one that finished does not run again. What it copies is synced to disk
before it records itself done. It refuses, before copying anything, packages
whose folders would be one folder on the home's drive, such as two names that
differ only in case where the drive ignores case.

## Auth (`$DATA_DIR/.auth/`)

*All components.* Managed by quilt-rs; CLI and QuiltSync use it through the
library. Per-host credentials, one subdirectory per catalog.
Wiped by **Debug → Erase auth** (QuiltSync).

| Artifact | Path | Path source |
|----------|------|-------------|
| OAuth client registration | `$DATA_DIR/.auth/<host>/client.json` | Host from the catalog URL at login |
| OAuth tokens | `$DATA_DIR/.auth/<host>/tokens.json` | Host from the catalog URL at login |
| AWS credentials | `$DATA_DIR/.auth/<host>/credentials.json` | Host from the catalog URL at login |
| Auth lock | `$DATA_DIR/.auth/.locks/<host>` | Created by its first holder |

### The store as a contract

Another client may share this store, and can rely on what follows.

**Location.** `$DATA_DIR` is the platform path in the table above. The CLI
uses the same one unless `--domain` or `QUILT_DOMAIN` names another. `<host>`
is the catalog's host name alone, with no scheme or port, lowercased:
`open.quiltdata.com`. A name typed at login is normalized as quiltx does, and kept
as typed otherwise, so a Unicode name is a folder named with that Unicode name
(`bücher.example`), not its punycode form. A folder written by an earlier version
for such a name is still named in punycode (`xn--bcher-kva.example`), and is still
read; a client that shares the store should look for both spellings of a Unicode
name. A folder named with an IP address, written by an earlier version, is also
still read.

Only signing in is strict about the name. The places that name a catalog that
already exists (`quilt push --host` and `--origin`, `quilt role --host`, a
package's remote, installing from a catalog, QuiltSync's bucket form) read it
as before and still take an IP address, so a stored account on an IP address
keeps working. It cannot be signed in to again by name: once its refresh token
expires, the account cannot be renewed through sign-in (a known limitation).

`.auth/.locks/` holds the locks, not a host.

**Files.** Each is one JSON object. Timestamps are RFC 3339 strings in UTC.

| File | Fields |
|------|--------|
| `tokens.json` | `access_token`, `refresh_token`, `expires_at` (of the access token) |
| `credentials.json` | `access_key`, `secret_key`, `token` (the STS session token), `expires_at` |
| `client.json` | `client_id`, `redirect_uri` (of the OAuth client registered for this host) |

quilt-rs refreshes the access token once it is within 60 seconds of
`expires_at`, and treats credentials as gone once `expires_at` has passed.

**Writes.** A file is written whole to a temp file in the same directory,
`.tmp-<uuid>`, then renamed over its target, so a reader never sees half a
file.

**Modes.** On Unix, `.auth/`, `.auth/.locks/` and each `<host>/` are created
0700, and the three files and the lock files 0600; a temp file is 0600 before
anything is written to it. Each write, and each lock, first sets the
directories above it to 0700 where the file system allows, so a store left
open by an earlier version is closed at its next write, and each file is
replaced with a 0600 one when it is next written. On Windows modes are
not set: the store relies on the per-user permissions of `%LOCALAPPDATA%`.

**The auth lock.** Refresh tokens rotate, so two clients refreshing one host
at once would leave one of them holding a token the other retired. To
refresh, a client:

1. takes an exclusive lock on `.auth/.locks/<host>`, creating the file
   and its directories if they are missing, and waits while another client
   holds it;
2. reads `tokens.json` again, since the last holder may have just refreshed
   it, and refreshes only if the access token it finds is within 60 seconds of
   expiring or the server has refused it;
3. writes the new tokens, and any `credentials.json` it vends with them, then
   releases the lock.

The lock is `flock(2)` with `LOCK_EX` on macOS and Linux (in Python,
`fcntl.flock`; `fcntl` and `lockf` record locks do not see it on Linux), and
`LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK` over the whole file from offset 0 on
Windows. The file holds no data, and is named by the host alone so it fits
wherever `<host>/` does. Never rename, truncate or delete it: a lock on a
replaced file guards nothing. It lives beside `<host>/` rather than in it
because logging out removes `<host>/` whole, under the same lock, so a
refresh waiting on it finds no `tokens.json` afterwards and writes nothing.
quilt-rs takes no lock for a host with no `<host>/`, which has no session to
guard. Logging in writes `tokens.json` without the lock.

## Shared settings (`$QUILT_AUTH_DIR/settings.json`)

*The `quilt` CLI only, so far.* Non-secret settings every Quilt client may
share, designed in
[quilt-specs#46](https://github.com/quiltdata/quilt-specs/pull/46). The folder
is `QUILT_AUTH_DIR` when set, else `com.quiltdata.auth` under the platform's
local data directory; it is not `$DATA_DIR`. On Unix a change sets the folder
to 0700 where the file system allows, an existing `QUILT_AUTH_DIR` folder
included.

| Artifact | Path | Contents |
|----------|------|----------|
| Settings | `<folder>/settings.json` | One JSON object; `default_catalog` is a bare catalog host, as in `.auth/<host>` |
| Settings lock | `<folder>/settings.json.lock` | Held by every writer; created by its first holder |

A missing file or key means there is no default catalog, unless quilt3 has
one: the first read takes the host of `navigator_url` from quilt3's
`config.yml`, which is only read, and stores it. Clearing the default stores
`null`, which stops that. A writer takes the lock, rereads the file and writes
it whole through a temp file and a rename, changing only its own key, so keys
it does not know survive. A file that is not a JSON object is reported and
never replaced. The default is only a fallback for a command with no package
and no host: a package's own catalog wins, and a package with no catalog stays
on this computer's AWS credentials.

## Logs (`$DATA_DIR/logs/`)

*QuiltSync only.* Opened by **Debug → Show logs**. Path is hardcoded relative
to `$DATA_DIR`.

## Remote S3 artifacts

*All components.* Written and read by quilt-rs during push/pull operations.

| S3 key | Description |
|--------|-------------|
| `s3://<bucket>/.quilt/packages/<hash>` | JSONL package manifests |
| `s3://<bucket>/.quilt/named_packages/<ns>/<tag>` | Tag pointers (e.g. `latest`) |
| `s3://<bucket>/.quilt/workflows/config.yml` | Push workflows and metadata schemas |

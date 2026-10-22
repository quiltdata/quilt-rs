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
`$DATA_DIR/.quilt/data.json` and read from there on subsequent starts.
Defaults to `~/QuiltSync` if the user has not configured it yet.
*CLI/library*: the caller provides the path.

## Domain (`$DATA_DIR/.quilt/data.json`)

*All components (quilt-rs, quilt-cli, QuiltSync).* Tracks all installed
packages, their remote origins, local commit state, and the home directory
path. Path is hardcoded relative to `$DATA_DIR`.

## Package state (`$DATA_DIR/.quilt/`)

*All components.* Managed by quilt-rs; CLI and QuiltSync use it through the
library.

| Artifact | Path | Path source |
|----------|------|-------------|
| Cached remote manifests | `$DATA_DIR/.quilt/packages/<bucket>/<hash>` | Bucket and hash from the remote manifest URI |
| Installed manifests | `$DATA_DIR/.quilt/installed/<ns>/<hash>` | Namespace from the package, hash from the manifest |
| Object store | `$DATA_DIR/.quilt/objects/<sha256hex>` | Hash derived from file content |

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
is the catalog's host name alone, with no scheme or port, lowercased and
IDNA-encoded as the WHATWG URL standard parses a host: `open.quiltdata.com`.
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

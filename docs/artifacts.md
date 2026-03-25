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

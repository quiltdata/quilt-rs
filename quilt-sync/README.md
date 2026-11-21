# Cross-platform desktop application for editing Quilt data packages

**QuiltSync keeps your data under revision control and never asks you to
think about it.**

Built for bench scientists rather than engineers, QuiltSync lets you find a
data package in the Quilt Web Catalog, open it on your desktop, and bring
down only the files you need — not the whole package — into an ordinary
local folder.

Edit them in your usual tools and QuiltSync syncs your changes back as a new
revision, so getting data in and out never means the command line or
uploading file copies through the browser.

Every revision is an immutable, reproducible snapshot with its metadata —
versioned data you can trust and return to, not the opaque mirroring of
ordinary file-sync.

Your team's existing rules still apply: each revision conforms to the
workflow configured for its bucket and reuses the same roles and policies
you already have in the Catalog — nothing new to set up.

## Usage

From the repository root:

```bash
npm install
npm start
```

### Testing deep linking on macOS

NOTE: macOS does not support dynamic deep linking. You need to build the app
and copy it to the Applications folder to test the deep linking feature.

```bash
npm run build # building dmg will fail
sudo cp -rf src-tauri/target/release/bundle/macos/QuiltSync.app ~/Applications
open ~/Applications/QuiltSync.app
```

## Upgrading: the dataspace cutover

QuiltSync now runs on the dataspace engine by default. Its first start imports,
once, the packages an older QuiltSync kept into the dataspace Domain, which it
finds as `quilt` does (`QUILT_DOMAIN`, else `bio.quilt.dataspace` in the OS's
local data folder). The old home becomes the default dataspace. QuiltSync
logs what it imported and shows it in a notice.

The old `.quilt/` folder is only read, so nothing is lost. An import that is
refused or stops is logged and shown, and QuiltSync runs on the ref engine
until a later start finishes it.

To keep the ref engine for one more release, put
`"dataspace_opt_out": true` in `experimental_settings.json` in the app's data
folder and restart QuiltSync. The `"dataspace"` key earlier builds read is
ignored.

## Repository Contents

### src: Static Web Frontend

This is the actual user interface presented by the application.

- index.html
- web assets
- styles.css

### src-tauri: Rust Backend

- Cargo.toml
- src/main.rs
- tauri.conf.json
- icons (for app icon)
- target (for output)

## Development

```bash
npm install typescript -g  # if `tsc` is not installed (need for git pre-commit)
cargo install cargo-watch  # only do once
RUST_BACKTRACE=1 cargo watch -C src-tauri  # -x test
```

### `parcel watch` and "Safe write"

`parcel watch` can fail on saving files if "Safe Write" is enabled in your IDE.

Learn more at <https://parceljs.org/features/development/#safe-write>.

## Auto Updater

QuiltSync automatically checks for updates on startup
and prompts users when new versions are available.

## Contributing

For maintainers and contributors, see [CONTRIBUTING.md](CONTRIBUTING.md) for testing
procedures, release processes, and development workflows.

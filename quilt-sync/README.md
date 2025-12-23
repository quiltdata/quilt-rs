# QuiltSync

## Cross-platform desktop application for editing Quilt data packages

Uses Tauri (<https://tauri.app/v1/guides/getting-started/setup/html-css-js>)
to build a Rust application with a web frontend.

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

## Contributing

For maintainers and contributors, see [CONTRIBUTING.md](CONTRIBUTING.md) for testing
procedures, release processes, and development workflows.

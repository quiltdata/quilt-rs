# A module recipe cannot depend on another module's recipe, so this one loads
# the gallery's file again, unlisted, for the UI stubs it creates.
[private]
mod gallery "ui/gallery.just"

# Run QuiltSync in development mode, rebuilding on save
dev: gallery::stubs
    cargo tauri dev

# Run QuiltSync headless in development mode, rebuilding on save, e.g. `--port 0 --data-dir /tmp/q`
headless *args: gallery::stubs
    cargo tauri dev -- -- --headless {{ args }}

# A module recipe cannot depend on another module's recipe, so this one loads
# the gallery's file again, unlisted, for the UI stubs it creates.
[private]
mod gallery "ui/gallery.just"

# Run QuiltSync in development mode, rebuilding on save
dev: gallery::stubs
    cargo tauri dev

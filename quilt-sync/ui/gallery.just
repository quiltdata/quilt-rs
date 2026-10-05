# GitHub Pages serves a project site under `/<repo>/`.
public_url := "/quilt-rs/"

# `--dist` keeps it apart from the app's `dist`: Trunk writes any target to
# `<dist>/index.html`, so a shared directory serves whichever rebuilt last.
# The port goes through Trunk's env var so `--port` overrides it.
#
# Serve a debug build and rebuild on save; `--open` opens a browser tab
dev *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8787}" trunk serve gallery.html --dist dist-gallery {{args}}

# Trunk 0.21 reads a bare `--release` followed by another argument as taking a
# value, hence `--release=true`.
#
# Write the release site to `dist-pages` (`public_url` defaults to `/quilt-rs/`)
build public_url=public_url: stubs
    trunk build --release=true gallery.html --public-url "{{public_url}}" --dist dist-pages

# Serve a release build as Pages does, rebuilding on save, at http://127.0.0.1:8787/quilt-rs/
preview *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8787}" trunk serve --release=true gallery.html --public-url "{{public_url}}" --dist dist-pages {{args}}

# Trunk refuses to start until the files in its watch-ignore list exist, which
# its pre-build hooks write. Empty stubs get it past that check.
[private]
stubs:
    mkdir -p assets/js && touch assets/js/json-editor.js assets/css/kit/_modules.scss assets/css/kit/_normalize.scss

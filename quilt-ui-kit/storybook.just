# GitHub Pages serves the storybook under the app gallery's site.
public_url := "/quilt-rs/kit/"

# The port goes through Trunk's env var so `--port` overrides it.
#
# Serve a debug build and rebuild on save; `--open` opens a browser tab
dev *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8788}" trunk serve {{args}}

# Trunk 0.21 reads a bare `--release` followed by another argument as taking a
# value, hence `--release=true`.
#
# Write the release site to `dist-pages` (`public_url` defaults to `/quilt-rs/kit/`)
build public_url=public_url: stubs
    trunk build --release=true --public-url "{{public_url}}" --dist dist-pages

# Serve a release build as Pages does, rebuilding on save, at http://127.0.0.1:8788/quilt-rs/kit/
preview *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8788}" trunk serve --release=true --public-url "{{public_url}}" --dist dist-pages {{args}}

# Trunk refuses to start until the files in its watch-ignore list exist, which
# its pre-build hook writes. An empty stub gets it past that check.
[private]
stubs:
    touch css/kit-modules.css

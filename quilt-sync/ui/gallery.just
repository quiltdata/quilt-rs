# The component gallery (`gallery.html`): the kit's design record, never the app.
# Needs the frontend toolchain, see CONTRIBUTING.md.
#
# The verbs mean what they mean in Vite:
#   dev      serve a debug build, rebuild on save (plain `just gallery` runs it)
#   build    write the release site to `dist-pages` and exit
#   preview  serve exactly what `build` writes, rebuilding on save
#
# Recipes here run in `quilt-sync/ui`, this file's directory.

# The subpath GitHub Pages serves a project site under: `/<repo>/`.
public_url := "/quilt-rs/"

# Use it while iterating: it rebuilds fast and keeps full names in panics, but
# its wasm is large. `preview` is the final look before pushing.
#
# `--dist` is load-bearing. Trunk writes whatever target it is given to
# `<dist>/index.html`, and `just start` points Tauri at `ui/dist`, so sharing one
# directory means whichever of the two rebuilt last owns the page both of them serve.
# With its own directory the gallery and the app run side by side.
#
# Extra arguments go to `trunk serve`: `just gallery dev --open`,
# `just gallery dev --address 0.0.0.0`. The port defaults through Trunk's env var
# rather than `--port`, so `just gallery dev --port 9000` overrides it instead of
# passing the flag twice.
#
# Serve a debug build and rebuild on save; `--open` opens a browser tab
dev *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8787}" trunk serve gallery.html --dist dist-gallery {{args}}

# Every URL Trunk writes takes the `public_url` prefix, so the files work only
# when served under it. Trunk 0.21 reads a bare `--release` followed by another
# argument as taking a value, hence `--release=true`. deploy-gallery.yaml runs
# this and publishes `dist-pages`.
#
# Write the release site to `dist-pages` and exit (`public_url` defaults to `/quilt-rs/`)
build public_url=public_url: stubs
    trunk build --release=true gallery.html --public-url "{{public_url}}" --dist dist-pages

# Release mode, the same `public_url` and the same `dist-pages` as `build`, so
# the page is served under the subpath as on GitHub Pages. Each build is slower
# than `dev`'s, but its wasm is as small as the shipped app's. Use it for a
# final look before pushing. Extra arguments go to `trunk serve`, as for `dev`.
#
# Trunk prints the subpath URL; the bare `http://127.0.0.1:8787/` is a 404, as
# is `/quilt-rs` without the trailing slash.
#
# Serve what `build` writes, rebuilding on save, at http://127.0.0.1:8787/quilt-rs/
preview *args: stubs
    TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8787}" trunk serve --release=true gallery.html --public-url "{{public_url}}" --dist dist-pages {{args}}

# Trunk refuses to start until the generated files in its watch-ignore list exist,
# so a fresh checkout fails before the pre-build hooks that write them ever run.
# Empty stubs get it past that check; the hooks then overwrite them. The root
# justfile's `ui-stubs` runs this one for `just start`.
[private]
stubs:
    mkdir -p assets/js && touch assets/js/json-editor.js assets/css/kit/_modules.scss assets/css/kit/_normalize.scss

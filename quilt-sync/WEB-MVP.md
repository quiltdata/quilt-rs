# QuiltSync in a browser (MVP)

A throwaway experiment: the QuiltSync UI runs in a plain browser and talks to
the backend over a WebSocket. The backend runs either inside the desktop app or
headless, with no window and no Tauri.

## Build the UI

The server serves the Trunk output folder, so build it first:

```sh
cd quilt-sync/ui && trunk build
```

The server looks in `quilt-sync/ui/dist` (resolved at compile time). Set
`QUILTSYNC_UI_DIR=/path/to/dist` to serve another folder, for example on a
machine that doesn't have the source tree.

## Headless

```sh
cargo run -p quilt-sync -- --headless \
  [--bind 127.0.0.1] [--port 7878] [--data-dir <path>] [--reset-token]
```

It prints `QuiltSync headless: http://localhost:7878/?t=<token>`. Open that
URL; the UI keeps the token for the session. Ctrl-C stops it. Started again on
the same data folder and port, it prints the same link, and an open tab
reconnects by itself (see [Restarts](#restarts)).

`cargo run` serves the UI already in `ui/dist`, so run `trunk build` first. For
development, `just app headless [args]` builds the UI and rebuilds it on save,
as `just app dev` does; reload the tab to see a change.

- `--bind <ip>`: the address to listen on, `127.0.0.1` by default. With
  `0.0.0.0` the server warns that everything, the token included, travels as
  plain HTTP. For a remote machine, prefer the default bind and an SSH tunnel:
  `ssh -L 7878:localhost:7878 <host>`, then open the printed URL locally.
- `--port <n>`: `7878` by default; `0` picks a free port.
- `--data-dir <path>`: the data folder. By default it is the desktop app's
  (`~/Library/Application Support/com.quiltdata.quilt-sync` on macOS). The
  environment variable `QUILTSYNC_DATA_DIR` sets it too, in both modes
  (`--data-dir` wins). Use a temp folder for tests and experiments.
- `--reset-token`: make a new token before starting, so every link handed
  out before stops working.

## Restarts

The token lives in `<data dir>/web-token` (mode 0600, written to a temp file
and renamed), made on the first start and reused after that by the headless
run and the desktop app alike. Only the engine that holds the engine lock
reads or makes it; a desktop app that failed to take the lock uses a fresh
token for that run.

- **Ports.** Headless keeps `7878` unless `--port` says otherwise. The
  desktop app remembers its last port in `<data dir>/web-port`, tries it
  first, falls back to a random port when it is taken, and saves the new one.
- **Reconnects.** When the socket drops, the page retries with the same token
  (back-off from 0.5 s up to 10 s). On reopen it re-runs every listener's
  `on_registered` read and bumps `tauri::reconnects()`, a tracked counter the
  main page's reads and the v2 page scaffold (`PageHandle::read`) follow, so
  they read again without a page reload.
- **A reset token.** A socket that closes before opening makes the page ask
  `GET /ws?t=<token>` over plain HTTP, since the browser hides the upgrade's
  status. A 401 means the token was reset: the page stops retrying, fails its
  requests, logs a console error and shows one banner, "QuiltSync was
  restarted with a new link. Open the link it printed."
- **Resetting.** `--reset-token` (headless), or the shared command
  `reset_web_token`, which writes a new token to `web-token`. It takes effect
  at the next start: the running server keeps the token it started with.
  There is no UI for it.

## One engine per data folder

Every engine, desktop or headless, holds an exclusive OS lock (`flock` /
`LockFileEx`) on `<data dir>/engine.lock` while it runs. Only the lock
counts, never whether the file exists: the file stays behind and is never
deleted, and after a crash or `kill -9` the OS has released the lock, so the
next start takes it. While holding it, an engine whose server is up writes
`<data dir>/engine.json` (`{pid, url, token, version}`, mode 0600); a reader
trusts it only while the lock is held by someone else.

- A second headless start on the same folder prints `QuiltSync is already
  running: <url>` and exits with code 2. When the running engine is the
  desktop app with no server yet, the address is unknown and it says so.
- A desktop start while a headless engine holds the folder opens the browser
  at that engine and exits. A second desktop app is still stopped by the
  single-instance plugin.
- The desktop app takes the lock before it starts its services.
  `engine_lock::publish(url, token)` writes `engine.json` once a server is
  up.

## Host check

On a loopback bind, every request (files and `/ws`) whose `Host` is not
`127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>` gets 403, which
defeats DNS rebinding. On any other bind (`--bind 0.0.0.0`) the check is off
and the headless run says so.

## The desktop app

The desktop app starts the server at launch, on its last port (or a random
one) with the saved token (see [Restarts](#restarts)), and its window loads
`http://127.0.0.1:<port>/?t=<token>`: the window is a page of the server and
talks to the backend over the same WebSocket as a browser, not over Tauri
IPC. The window keeps the label `main`; it is built in `setup` (`main.rs`),
not in `tauri.conf.json`.

- **WebSocket mode** is chosen by the token alone (`tauri::is_web_mode` in the
  UI): a page with a token uses the socket even when Tauri injects
  `window.__TAURI__` into it (`withGlobalTauri` stays on). The bundled
  `tauri://` page has no token and keeps IPC.
- **Desktop-only commands** go through `dispatch` too: `Services::desktop`
  holds the app's handle once setup ran, and with it `login_oauth`,
  `open_directory_picker` (the native picker over the main window),
  `check_for_update`, `download_and_install_update`, `quit_confirm` and
  `copy_to_clipboard` do what their Tauri commands do. The quit prompt's event
  goes out on the event sink. A headless run has no handle and answers as
  below.
- **Navigation** from the backend (a `quilt+s3://` deep link, the page after
  an OAuth login) is a `navigate` event, which the window and any browser
  follow. A startup deep link waits until a page is connected to the socket.
- **Open in browser** in the tray menu opens the same address in the default
  browser. The server stops when the app quits. The browser and the window
  share one backend, so both see every event.
- `cargo tauri dev` still works: `beforeDevCommand` runs `trunk watch` into
  `ui/dist`, which the server serves; reload the window to pick up a rebuild.
- `QUILTSYNC_IPC_WINDOW=1` loads the bundled page over IPC instead, to compare
  the two.

### A throwaway data folder

`QUILTSYNC_DATA_DIR=/some/dir` points the desktop app and the headless run at
another data folder. With it the desktop app skips the single-instance plugin,
so it runs next to an installed QuiltSync instead of handing itself over to it.

### Timing run

`QUILTSYNC_BENCH=<namespace>` makes the window time the biggest reads
(`get_package_page_data`, `get_installed_package_data`,
`get_main_page_packages`, `get_installed_packages_list_data`, ten runs each,
from the UI's call to the deserialized value) and the package page's render
(navigation to the frame after its context pane appears, five runs), then
append the results to `bench-report.txt` in the data folder. The server also
logs each socket command's run and serialize time.

## Protocol

`GET /ws?t=<token>` upgrades to a WebSocket. A missing or wrong token gets 401;
an `Origin` header whose host and port differ from `Host` gets 403. Requests
are `{"id", "cmd", "args"}` with the arguments under their camelCase names, as
Tauri's `invoke` takes them; replies are `{"id", "ok"}` or `{"id", "err"}`.
Every backend event is pushed as `{"event", "payload"}`. Every other path
serves a file from the UI folder, or `index.html` when there is none.

## Choosing a folder

Every page that asks for a folder (Setup, Create package, v1 and v2) calls
`pages::pick_folder`. It tries the native picker first; when that answers
`not available in the browser` (a headless engine), it opens an in-page folder
browser (`pages/folder_browser.rs`, mounted once in `App`) fed by the shared
command `list_folders(path)`: `{path, parent, folders, home}`, the home folder
when `path` is empty, visible subfolders only. The path field takes a typed
path on Enter; Up, Home and a click on a folder move around. The desktop app
keeps its native picker, over IPC or the socket.

## End-to-end tests

`quilt-sync/e2e` drives the real app in Chromium with Playwright:

```sh
just test e2e            # builds, then runs every test
just test e2e --headed   # extra arguments go to `playwright test`
```

The recipe builds the debug binary and the UI, installs the pinned
`@playwright/test` and its Chromium, and runs the suite. Each test starts its
own `quilt-sync --headless --port 0` on a fresh temp folder (data folder, home
folder and `HOME`), sets the home with the `setup` command over the WebSocket
(quicker than the folder browser), and stops it with Ctrl-C at
the end. `quiltsync.restart()` stops it and starts it again on the same data
folder and port (with `--reset-token` when asked); `restart.spec.ts` uses it
to check that an open page reconnects and re-reads, and that a reset token
shows the banner. A debug build sends no telemetry. To run the suite without the
recipe, build first, then `npx playwright test` in `quilt-sync/e2e`;
`QUILTSYNC_BIN` points it at another binary. It is not part of `just ci`.

## Known limits

- Plain HTTP only, one token per data folder until reset, no expiry, no rate
  limit.
- Only the main page and the v2 pages read again after a reconnect; the v1
  pages pick up only what their `on_registered` reads cover.
- Desktop-only commands (OAuth login, the folder picker, the clipboard,
  installing an update, confirming a quit) answer
  `not available in the browser`; the UI uses its own folder browser for
  the picker. The update check reads the updater feed
  (`https://www.quilt.bio/hubfs/latest.json`, copied from `tauri.conf.json`)
  and the notice shows a newer version, but its install button fails.
- Headless, the desktop-only commands (OAuth login, the folder picker, the
  clipboard, installing an update, confirming a quit) answer
  `not available in the browser`; the UI copies with the browser's
  clipboard and picks folders with its own folder browser instead; the
  update check answers "no update".
- The folder browser lists the engine's machine, not the browser's, and
  anyone with the token can list any folder the engine can read.
- Commands that open things (a folder, a file, the catalog) run on the
  machine QuiltSync runs on, not in the browser.
- On Windows release builds the console is hidden, so the headless URL is not
  printed.
- The UI folder path is baked in at compile time unless `QUILTSYNC_UI_DIR` is
  set.
- The file watcher takes up a new package on its next reconcile, every 5
  seconds, and does not walk it then, so a file written into a package just
  after it was created does not show until the next change. The end-to-end
  tests write again until the file shows; the first write is missed every
  time.

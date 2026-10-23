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
  [--bind 127.0.0.1] [--port 7878] [--data-dir <path>]
```

It prints `QuiltSync headless: http://localhost:7878/?t=<token>`. Open that
URL; the UI keeps the token for the session. Ctrl-C stops it.

- `--bind <ip>`: the address to listen on, `127.0.0.1` by default. With
  `0.0.0.0` the server warns that everything, the token included, travels as
  plain HTTP. For a remote machine, prefer the default bind and an SSH tunnel:
  `ssh -L 7878:localhost:7878 <host>`, then open the printed URL locally.
- `--port <n>`: `7878` by default; `0` picks a free port.
- `--data-dir <path>`: the data folder. By default it is the desktop app's
  (`~/Library/Application Support/com.quiltdata.quilt-sync` on macOS). The
  environment variable `QUILTSYNC_DATA_DIR` sets it too, in both modes
  (`--data-dir` wins). Use a temp folder for tests and experiments.

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
  up; only the headless run calls it so far.

## Host check

On a loopback bind, every request (files and `/ws`) whose `Host` is not
`127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>` gets 403, which
defeats DNS rebinding. On any other bind (`--bind 0.0.0.0`) the check is off
and the headless run says so.

## The desktop app

The desktop app starts the server at launch, on a random local port with a
fresh token, and its window loads `http://127.0.0.1:<port>/?t=<token>`: the
window is a page of the server and talks to the backend over the same
WebSocket as a browser, not over Tauri IPC. The window keeps the label
`main`; it is built in `setup` (`main.rs`), not in `tauri.conf.json`.

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
(the Setup page needs the native folder picker), and stops it with Ctrl-C at
the end. A debug build sends no telemetry. To run the suite without the
recipe, build first, then `npx playwright test` in `quilt-sync/e2e`;
`QUILTSYNC_BIN` points it at another binary. It is not part of `just ci`.

## Known limits

- Plain HTTP only, one token per run, no expiry, no rate limit.
- Desktop-only commands (OAuth login, the folder picker, the clipboard,
  installing an update, confirming a quit) answer
  `not available in the browser`. The update check reads the updater feed
  (`https://www.quilt.bio/hubfs/latest.json`, copied from `tauri.conf.json`)
  and the notice shows a newer version, but its install button fails.
- Headless, the desktop-only commands (OAuth login, the folder picker, the
  clipboard, installing an update, confirming a quit) answer
  `not available in the browser`, and the UI copies with the browser's
  clipboard instead; the update check answers "no update".
- Commands that open things (a folder, a file, the catalog) run on the
  machine QuiltSync runs on, not in the browser.
- On Windows release builds the console is hidden, so the headless URL is not
  printed.
- The UI folder path is baked in at compile time unless `QUILTSYNC_UI_DIR` is
  set.

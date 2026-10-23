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
cargo run -p quilt-sync -- --headless [--bind 127.0.0.1] [--port 7878] [--data-dir <path>]
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

## From the desktop app

The tray menu has **Open in browser**. The first click starts the server on a
random local port with a fresh token and opens it in the default browser; later
clicks reopen the same address. The server stops when the app quits. The
browser and the window share one backend, so both see every event.

## Protocol

`GET /ws?t=<token>` upgrades to a WebSocket. A missing or wrong token gets 401;
an `Origin` header whose host and port differ from `Host` gets 403. Requests
are `{"id", "cmd", "args"}` with the arguments under their camelCase names, as
Tauri's `invoke` takes them; replies are `{"id", "ok"}` or `{"id", "err"}`.
Every backend event is pushed as `{"event", "payload"}`. Every other path
serves a file from the UI folder, or `index.html` when there is none.

## Known limits

- Plain HTTP only, one token per run, no expiry, no rate limit.
- Desktop-only commands (OAuth login, the folder picker, the clipboard,
  installing an update, confirming a quit) answer
  `not available in the browser`. The update check reads the updater feed
  (`https://www.quilt.bio/hubfs/latest.json`, copied from `tauri.conf.json`)
  and the notice shows a newer version, but its install button fails.
- Commands that open things (a folder, a file, the catalog) run on the
  machine QuiltSync runs on, not in the browser.
- On Windows release builds the console is hidden, so the headless URL is not
  printed.
- The UI folder path is baked in at compile time unless `QUILTSYNC_UI_DIR` is
  set.

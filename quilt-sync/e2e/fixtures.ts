// One headless QuiltSync per test, on throwaway folders.
//
// The binary and the UI are built beforehand (`just test e2e` does it). A
// debug build sends no telemetry: analytics only print a dry-run line and no
// crash reporter is built.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test as base } from "@playwright/test";

const ROOT = resolve(import.meta.dirname, "../..");
const BINARY = process.env.QUILTSYNC_BIN ?? join(ROOT, "target/debug/quilt-sync");
const UI_DIR = process.env.QUILTSYNC_UI_DIR ?? join(ROOT, "quilt-sync/ui/dist");

export interface QuiltSync {
  /** `http://localhost:<port>`, no token. */
  origin: string;
  token: string;
  /** The folder packages live in, set as the home before the test starts. */
  home: string;
  /** An address on this server with the token, e.g. `url("/")`. */
  url(path: string): string;
  /** Run a backend command over the WebSocket, as the UI does. */
  call<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

/** Start `quilt-sync --headless` and wait for the line with its address. */
async function start(dataDir: string, fakeHome: string, log: string[]) {
  const child = spawn(BINARY, ["--headless", "--port", "0", "--data-dir", dataDir], {
    env: {
      ...process.env,
      // Nothing the backend reads from the user's home (AWS profiles, ignore
      // rules) may leak in.
      HOME: fakeHome,
      QUILTSYNC_UI_DIR: UI_DIR,
      QUILTSYNC_DATA_DIR: dataDir,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const address = await new Promise<URL>((ok, fail) => {
    let out = "";
    const timer = setTimeout(() => fail(new Error(`no address after 30s:\n${log.join("")}`)), 30_000);
    child.stdout!.on("data", (chunk: Buffer) => {
      log.push(chunk.toString());
      out += chunk.toString();
      const found = out.match(/QuiltSync headless: (http:\/\/\S+)/);
      if (found) {
        clearTimeout(timer);
        ok(new URL(found[1]));
      }
    });
    child.stderr!.on("data", (chunk: Buffer) => log.push(chunk.toString()));
    child.on("exit", (code) => {
      clearTimeout(timer);
      fail(new Error(`quilt-sync exited with ${code}:\n${log.join("")}`));
    });
    child.on("error", fail);
  });
  return { child, address };
}

function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((done) => {
    const kill = setTimeout(() => child.kill("SIGKILL"), 5_000);
    child.once("exit", () => {
      clearTimeout(kill);
      done();
    });
    // Ctrl-C is how the headless run is meant to stop.
    child.kill("SIGINT");
  });
}

/** One request over a socket of its own. */
async function call<T>(origin: string, token: string, cmd: string, args: Record<string, unknown>): Promise<T> {
  const socket = new WebSocket(`${origin.replace(/^http/, "ws")}/ws?t=${token}`);
  try {
    await new Promise((ok, fail) => {
      socket.addEventListener("open", ok, { once: true });
      socket.addEventListener("error", () => fail(new Error(`cannot connect for ${cmd}`)), { once: true });
    });
    return await new Promise<T>((ok, fail) => {
      socket.addEventListener("message", (ev) => {
        const msg = JSON.parse(String(ev.data));
        if (msg.id !== 1) return; // an event
        if ("ok" in msg) ok(msg.ok as T);
        else fail(new Error(`${cmd}: ${JSON.stringify(msg.err)}`));
      });
      socket.send(JSON.stringify({ id: 1, cmd, args }));
    });
  } finally {
    socket.close();
  }
}

export const test = base.extend<{ quiltsync: QuiltSync }>({
  quiltsync: async ({}, use, testInfo) => {
    // `realpath`: macOS's temp folder is behind a symlink, and the backend
    // reports resolved paths.
    const root = await realpath(await mkdtemp(join(tmpdir(), "quiltsync-e2e-")));
    const log: string[] = [];
    const { child, address } = await start(join(root, "data"), join(root, "user"), log);
    const origin = address.origin;
    const token = address.searchParams.get("t")!;
    const quiltsync: QuiltSync = {
      origin,
      token,
      home: join(root, "home"),
      url(path) {
        const url = new URL(path, origin);
        url.searchParams.set("t", token);
        return url.toString();
      },
      call: (cmd, args = {}) => call(origin, token, cmd, args),
    };
    try {
      // The Setup page asks for this folder with a native picker.
      await quiltsync.call("setup", { directory: quiltsync.home });
      await use(quiltsync);
    } finally {
      await stop(child);
      if (testInfo.status !== testInfo.expectedStatus) {
        await testInfo.attach("quilt-sync.log", { body: log.join(""), contentType: "text/plain" });
      }
      await rm(root, { recursive: true, force: true });
    }
  },
});

export { expect } from "@playwright/test";

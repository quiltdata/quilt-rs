// One `quilt-sync --headless` per test, on throwaway folders.
//
// The UI and the debug binary are built beforehand (`just test e2e` does
// both). A debug build sends no telemetry: analytics print a dry-run line.

import { type ChildProcess, spawn } from "node:child_process";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test as base } from "@playwright/test";

const ROOT = resolve(import.meta.dirname, "../..");
const BINARY =
  process.env.QUILTSYNC_BIN ?? join(ROOT, "target/debug/quilt-sync");

export interface QuiltSync {
  /** `http://127.0.0.1:<port>`, no token. */
  origin: string;
  token: string;
  /** The data folder, which the engine owns. */
  domain: string;
  /** The folder packages live in. */
  home: string;
  /** An address on this engine with the token, e.g. `url("/")`. */
  url(path: string): string;
  /** Run an engine command over the WebSocket, as the UI does. */
  call<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  /** Stop the engine with Ctrl-C and start it again on the same folder. */
  restart(): Promise<void>;
}

interface Options {
  /** Set the home folder over the socket first, past the Setup page. */
  setUp: boolean;
}

/** Start the engine and wait for the line with its link. */
async function start(domain: string, fakeHome: string, log: string[]) {
  const env = { ...process.env, HOME: fakeHome };
  // `--domain` wins anyway; this keeps the parent's from showing in logs.
  delete env.QUILT_DOMAIN;
  const child = spawn(BINARY, ["--headless", "--domain", domain], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const ready = new Promise<URL>((ok, fail) => {
    let out = "";
    const timer = setTimeout(
      () => fail(new Error(`no link after 30s:\n${log.join("")}`)),
      30_000,
    );
    child.stdout?.on("data", (chunk: Buffer) => {
      log.push(chunk.toString());
      out += chunk.toString();
      // A chunk can end inside the link, so only a whole line counts.
      const found = out.match(
        /QuiltSync is running\. Open (http:\/\/\S+)\r?\n/,
      );
      if (found) {
        clearTimeout(timer);
        ok(new URL(found[1]));
      }
    });
    child.stderr?.on("data", (chunk: Buffer) => log.push(chunk.toString()));
    child.on("exit", (code) => {
      clearTimeout(timer);
      fail(new Error(`quilt-sync exited with ${code}:\n${log.join("")}`));
    });
    child.on("error", fail);
  });
  try {
    return { child, link: await ready };
  } catch (err) {
    // An engine that never printed its link would otherwise outlive the run.
    await stop(child);
    throw err;
  }
}

function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null)
    return Promise.resolve();
  return new Promise((done) => {
    const kill = setTimeout(() => child.kill("SIGKILL"), 10_000);
    child.once("exit", () => {
      clearTimeout(kill);
      done();
    });
    // Ctrl-C is how a headless run is meant to stop.
    child.kill("SIGINT");
  });
}

/** One request over a socket of its own. */
async function call<T>(
  origin: string,
  token: string,
  cmd: string,
  args: Record<string, unknown>,
): Promise<T> {
  const socket = new WebSocket(
    `${origin.replace(/^http/, "ws")}/ws?t=${token}`,
  );
  try {
    await new Promise((ok, fail) => {
      socket.addEventListener("open", ok, { once: true });
      socket.addEventListener(
        "error",
        () => fail(new Error(`cannot connect for ${cmd}`)),
        { once: true },
      );
    });
    return await new Promise<T>((ok, fail) => {
      socket.addEventListener("message", (ev) => {
        const msg = JSON.parse(String(ev.data));
        // The hello and events carry no id.
        if (msg.id !== 1) return;
        if ("ok" in msg) ok(msg.ok as T);
        else fail(new Error(`${cmd}: ${JSON.stringify(msg.err)}`));
      });
      socket.send(JSON.stringify({ id: 1, cmd, args }));
    });
  } finally {
    socket.close();
  }
}

export const test = base.extend<Options & { quiltsync: QuiltSync }>({
  setUp: [true, { option: true }],
  quiltsync: async ({ setUp }, use, testInfo) => {
    // `realpath`: macOS's temp folder is behind a symlink, and the engine
    // reports resolved paths.
    const root = await realpath(
      await mkdtemp(join(tmpdir(), "quiltsync-e2e-")),
    );
    const domain = join(root, "domain");
    const fakeHome = join(root, "user");
    const log: string[] = [];
    let { child, link } = await start(domain, fakeHome, log);
    const quiltsync: QuiltSync = {
      origin: link.origin,
      token: link.searchParams.get("t") ?? "",
      domain,
      home: join(root, "home"),
      url(path) {
        const url = new URL(path, quiltsync.origin);
        url.searchParams.set("t", quiltsync.token);
        return url.toString();
      },
      call: (cmd, args = {}) =>
        call(quiltsync.origin, quiltsync.token, cmd, args),
      async restart() {
        await stop(child);
        ({ child, link } = await start(domain, fakeHome, log));
        quiltsync.origin = link.origin;
        quiltsync.token = link.searchParams.get("t") ?? "";
      },
    };
    try {
      if (setUp) await quiltsync.call("setup", { directory: quiltsync.home });
      await use(quiltsync);
    } finally {
      await stop(child);
      if (testInfo.status !== testInfo.expectedStatus) {
        await testInfo.attach("quilt-sync.log", {
          body: log.join(""),
          contentType: "text/plain",
        });
      }
      await rm(root, { recursive: true, force: true });
    }
  },
});

export { expect } from "@playwright/test";

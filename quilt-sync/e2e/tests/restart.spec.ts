import { rm } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "../fixtures";

test("an open page reconnects after QuiltSync restarts", async ({
  page,
  quiltsync,
}) => {
  await quiltsync.call("package_create", {
    namespace: "e2e/before",
    source: null,
    message: null,
  });
  // While `away`, the page's sockets close as they open, so it misses the
  // change below and sees it only by reading again once back.
  let away = false;
  await page.routeWebSocket(/\/ws/, (ws) => {
    if (away) ws.close();
    else ws.connectToServer();
  });
  await page.goto(quiltsync.url("/"));
  const packages = page.getByRole("region", { name: "Packages" });
  await expect(
    packages.getByRole("link", { name: "e2e/before" }).first(),
  ).toBeVisible();
  // A page that reloads loses this mark.
  await page.evaluate(() => Object.assign(window, { e2eMark: true }));

  const { origin, token } = quiltsync;
  away = true;
  await quiltsync.restart();
  expect(quiltsync.origin).toBe(origin);
  expect(quiltsync.token).toBe(token);
  await quiltsync.call("package_create", {
    namespace: "e2e/after",
    source: null,
    message: null,
  });
  await expect(page.getByText("Reconnecting")).toBeVisible();
  await expect(packages.getByRole("link", { name: "e2e/after" })).toHaveCount(
    0,
  );
  away = false;

  await expect(
    packages.getByRole("link", { name: "e2e/after" }).first(),
  ).toBeVisible({ timeout: 30_000 });
  await expect(
    packages.getByRole("link", { name: "e2e/before" }).first(),
  ).toBeVisible();
  expect(await page.evaluate(() => "e2eMark" in window)).toBe(true);
});

test("a page with the old link says to open the printed one", async ({
  page,
  quiltsync,
}) => {
  await page.goto(quiltsync.url("/"));
  await expect(
    page.getByRole("heading", { name: "No packages yet" }),
  ).toBeVisible();

  // Without its token file, the engine makes a new token as it starts.
  const token = quiltsync.token;
  await rm(join(quiltsync.domain, "web-token"));
  await quiltsync.restart();
  expect(quiltsync.token).not.toBe(token);

  await expect(
    page.getByText("Open the link it printed when it started."),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText("tray")).toHaveCount(0);
});

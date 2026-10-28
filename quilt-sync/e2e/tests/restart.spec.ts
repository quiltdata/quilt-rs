import { expect, test } from "../fixtures";

test("an open page reconnects after QuiltSync restarts", async ({ page, quiltsync }) => {
  await quiltsync.call("package_create", { namespace: "e2e/before", source: null, message: null });
  await page.goto(quiltsync.url("/"));
  const packages = page.getByRole("region", { name: "Packages" });
  await expect(packages.getByRole("link", { name: "e2e/before" })).toBeVisible();
  // A page that reloads loses this mark.
  await page.evaluate(() => ((window as any).e2eMark = true));

  const token = quiltsync.token;
  await quiltsync.restart();
  expect(quiltsync.token).toBe(token);
  // Made while the page is away, so it shows only if the page reads again.
  await quiltsync.call("package_create", { namespace: "e2e/after", source: null, message: null });

  // The attention queue links each package too.
  await expect(packages.getByRole("link", { name: "e2e/after" }).first()).toBeVisible({ timeout: 30_000 });
  await expect(packages.getByRole("link", { name: "e2e/before" }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).e2eMark)).toBe(true);
});

test("a page with the old link says so after a token reset", async ({ page, quiltsync }) => {
  await page.goto(quiltsync.url("/"));
  await expect(page.getByRole("heading", { name: "No packages yet" })).toBeVisible();

  const token = quiltsync.token;
  await quiltsync.restart({ resetToken: true });
  expect(quiltsync.token).not.toBe(token);

  await expect(page.getByRole("alert").filter({ hasText: "QuiltSync was restarted with a new link" })).toBeVisible({
    timeout: 30_000,
  });
});

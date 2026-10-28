import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type QuiltSync } from "../fixtures";

const NAMESPACE = "e2e/first";

/** Create a package through the main page's dialog, as a user would. */
async function createPackage(page: Page, namespace: string) {
  await page.getByRole("button", { name: "Create package" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("owner/package-name").fill(namespace);
  await dialog.getByRole("button", { name: "Create", exact: true }).click();
  // The dialog closes on success, and the list reads the new package.
  await expect(dialog).toBeHidden();
}

/**
 * Write `name` into the package's folder until every page lists it.
 *
 * The watcher picks up a new package on its next reconcile, a few seconds
 * after it is created, and a write before that goes unseen. Writing again
 * until the file shows covers that gap without knowing its length.
 */
async function writeUntilListed(quiltsync: QuiltSync, namespace: string, name: string, pages: Page[]) {
  const folder = await quiltsync.call<string>("package_file_path", { namespace, path: "" });
  // A page that reloads loses this mark.
  for (const page of pages) await page.evaluate(() => ((window as any).e2eMark = true));
  let round = 0;
  await expect(async () => {
    await writeFile(join(folder, name), `round ${++round}\n`);
    for (const page of pages) {
      await expect(fileRow(page, name)).toBeVisible({ timeout: 4_000 });
    }
  }).toPass({ timeout: 30_000 });
  test.info().annotations.push({ type: "writes", description: `${name}: ${round}` });
  for (const page of pages) expect(await page.evaluate(() => (window as any).e2eMark)).toBe(true);
}

/** The package page's header names the package. */
function packageHeading(page: Page, namespace: string) {
  return page.getByRole("heading", { name: namespace, exact: true });
}

/** A file's row in the package page's Files section. */
function fileRow(page: Page, name: string) {
  return page.getByRole("region", { name: "Files" }).getByText(name, { exact: true });
}

test("the main page renders after setup", async ({ page, quiltsync }) => {
  await page.goto(quiltsync.url("/"));
  await expect(page.getByRole("heading", { name: "No packages yet" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create package" }).first()).toBeVisible();
});

test("a file written into a package folder appears on its page", async ({ page, quiltsync }) => {
  await page.goto(quiltsync.url("/"));
  await createPackage(page, NAMESPACE);
  // The attention queue links the new package too; open it from the list.
  await page.getByRole("region", { name: "Packages" }).getByRole("link", { name: NAMESPACE }).click();
  await expect(packageHeading(page, NAMESPACE)).toBeVisible();

  // No reload: the watcher's event reaches the page over the socket.
  await writeUntilListed(quiltsync, NAMESPACE, "hello.txt", [page]);
});

test("two pages on one package both see a new file", async ({ browser, quiltsync }) => {
  await quiltsync.call("package_create", { namespace: NAMESPACE, source: null, message: null });
  const packagePage = quiltsync.url(`/installed-package?namespace=${encodeURIComponent(NAMESPACE)}`);

  // Two contexts: two separate browser sessions, each with its own socket.
  const pages = await Promise.all(
    [0, 1].map(async () => {
      const page = await (await browser.newContext()).newPage();
      await page.goto(packagePage);
      await expect(packageHeading(page, NAMESPACE)).toBeVisible();
      return page;
    }),
  );

  await writeUntilListed(quiltsync, NAMESPACE, "shared.txt", pages);
  await Promise.all(pages.map((page) => page.context().close()));
});

import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type QuiltSync } from "../fixtures";

const NAMESPACE = "e2e/first";

/** Create a package through the main page's dialog, as a user would. */
async function createPackage(page: Page, namespace: string) {
  await page.getByRole("button", { name: "Create local package" }).click();
  await page.getByPlaceholder("owner/package-name").fill(namespace);
  await page.getByRole("button", { name: "Create", exact: true }).click();
  // The result shows in a bar over the page until a click anywhere dismisses it.
  const notice = page.getByText(`Successfully created package ${namespace}`);
  await expect(notice).toBeVisible();
  const { width, height } = page.viewportSize()!;
  await page.mouse.click(width - 5, height - 5);
  await expect(notice).toBeHidden();
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
      await expect(page.getByTestId("entry-name").filter({ hasText: name })).toBeVisible({ timeout: 4_000 });
    }
  }).toPass({ timeout: 30_000 });
  test.info().annotations.push({ type: "writes", description: `${name}: ${round}` });
  for (const page of pages) expect(await page.evaluate(() => (window as any).e2eMark)).toBe(true);
}

/** The package page's breadcrumb ends with the namespace. */
function packageCrumb(page: Page, namespace: string) {
  return page.getByRole("navigation").getByText(namespace, { exact: true });
}

test("the main page renders after setup", async ({ page, quiltsync }) => {
  await page.goto(quiltsync.url("/"));
  await expect(page.getByRole("heading", { name: "You don't have any packages" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create local package" })).toBeVisible();
});

test("a file written into a package folder appears on its page", async ({ page, quiltsync }) => {
  await page.goto(quiltsync.url("/"));
  await createPackage(page, NAMESPACE);
  await page.getByRole("link", { name: NAMESPACE }).click();
  await expect(packageCrumb(page, NAMESPACE)).toBeVisible();

  // No reload: the watcher's event reaches the page over the socket.
  await writeUntilListed(quiltsync, NAMESPACE, "hello.txt", [page]);
});

test("two pages on one package both see a new file", async ({ browser, quiltsync }) => {
  await quiltsync.call("package_create", { namespace: NAMESPACE, source: null, message: null });
  const packagePage = quiltsync.url(`/installed-package?namespace=${encodeURIComponent(NAMESPACE)}&filter=unmodified`);

  // Two contexts: two separate browser sessions, each with its own socket.
  const pages = await Promise.all(
    [0, 1].map(async () => {
      const page = await (await browser.newContext()).newPage();
      await page.goto(packagePage);
      await expect(packageCrumb(page, NAMESPACE)).toBeVisible();
      return page;
    }),
  );

  await writeUntilListed(quiltsync, NAMESPACE, "shared.txt", pages);
  await Promise.all(pages.map((page) => page.context().close()));
});

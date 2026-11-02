import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, type QuiltSync, test } from "../fixtures";

const NAMESPACE = "e2e/first";

/** A file's row in the package page's Files section. */
function fileRow(page: Page, name: string) {
  return page
    .getByRole("region", { name: "Files" })
    .getByText(name, { exact: true });
}

/**
 * Write `name` into the package's folder until every page lists it.
 *
 * The watcher picks up a new package on its next reconcile, a few seconds
 * after it is created, and a write before that goes unseen. Writing again
 * until the file shows covers that gap without knowing its length.
 */
async function writeUntilListed(
  quiltsync: QuiltSync,
  namespace: string,
  name: string,
  pages: Page[],
) {
  const folder = await quiltsync.call<string>("package_file_path", {
    namespace,
    path: "",
  });
  // A page that reloads loses this mark.
  for (const page of pages)
    await page.evaluate(() => Object.assign(window, { e2eMark: true }));
  let round = 0;
  await expect(async () => {
    round += 1;
    await writeFile(join(folder, name), `round ${round}\n`);
    for (const page of pages) {
      await expect(fileRow(page, name)).toBeVisible({ timeout: 4_000 });
    }
  }).toPass({ timeout: 30_000 });
  for (const page of pages) {
    expect(await page.evaluate(() => "e2eMark" in window)).toBe(true);
  }
}

test("a package made in the dialog opens, and a new file appears on its page", async ({
  page,
  quiltsync,
}) => {
  await page.goto(quiltsync.url("/"));
  await page.getByRole("button", { name: "Create package" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("owner/package-name").fill(NAMESPACE);
  await dialog.getByRole("button", { name: "Create", exact: true }).click();
  await expect(dialog).toBeHidden();

  await page
    .getByRole("region", { name: "Packages" })
    .getByRole("link", { name: NAMESPACE })
    .first()
    .click();
  await expect(page).toHaveTitle(`${NAMESPACE} · QuiltSync`);

  // No reload: the watcher's event reaches the page over the socket.
  await writeUntilListed(quiltsync, NAMESPACE, "hello.txt", [page]);
});

test("two tabs on one package both see a new file", async ({
  browser,
  quiltsync,
}) => {
  await quiltsync.call("package_create", {
    namespace: NAMESPACE,
    source: null,
    message: null,
  });
  const packagePage = quiltsync.url(
    `/installed-package?namespace=${encodeURIComponent(NAMESPACE)}`,
  );

  // Two contexts: two browser sessions, each with its own socket.
  const pages = await Promise.all(
    [0, 1].map(async () => {
      const page = await (await browser.newContext()).newPage();
      await page.goto(packagePage);
      await expect(page).toHaveTitle(`${NAMESPACE} · QuiltSync`);
      return page;
    }),
  );

  await writeUntilListed(quiltsync, NAMESPACE, "shared.txt", pages);
  await Promise.all(pages.map((page) => page.context().close()));
});

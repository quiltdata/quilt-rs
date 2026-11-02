import { expect, test } from "../fixtures";

test.use({ setUp: false });

test("setup takes a typed folder and opens the main page", async ({
  page,
  quiltsync,
}) => {
  await page.goto(quiltsync.url("/"));
  await expect(
    page.getByRole("heading", { name: "Set up QuiltSync", level: 1 }),
  ).toBeVisible();
  // The link's token moved into the page and left the address.
  expect(new URL(page.url()).searchParams.has("t")).toBe(false);

  // Headless has no folder picker: the path is typed.
  const field = page.getByLabel("Set home directory");
  await field.fill(quiltsync.home);
  await expect(
    page.getByText("QuiltSync will make this folder."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save" }).click();

  await expect(
    page.getByRole("heading", { name: "No packages yet" }),
  ).toBeVisible();
  await expect(page).toHaveTitle("QuiltSync");
});

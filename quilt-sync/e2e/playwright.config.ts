import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  // Each test starts its own engine, so tests may run side by side.
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: [["list"]],
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: {
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      testIgnore: /restart\.spec\.ts/,
      use: { ...devices["Desktop Chrome"] },
    },
    // A restart gives up its port for a moment, and an engine starting
    // elsewhere could take it, so these run once the others are done.
    {
      name: "restart",
      testMatch: /restart\.spec\.ts/,
      dependencies: ["chromium"],
      fullyParallel: false,
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});

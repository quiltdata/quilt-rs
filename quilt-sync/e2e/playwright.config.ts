import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  // Each test starts its own backend, so tests may run side by side.
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: [["list"]],
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: {
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});

/**
 * Configures the Playwright visual test suite, preview URL, browser profile,
 * timing, viewport, and failure screenshot behavior.
 * Made by Heavymask — https://heavymask.com
 */
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./playwright",
  timeout: 45_000,
  fullyParallel: true,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    ...devices["Desktop Chrome"],
    viewport: { width: 300, height: 520 },
    screenshot: "only-on-failure",
  },
});

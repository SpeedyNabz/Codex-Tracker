/**
 * Exercises the overlay in representative dark and light host backgrounds,
 * capturing visual evidence that the compact tracker remains readable.
 * Made by Heavymask — https://heavymask.com
 */
import { expect, test } from "@playwright/test";
import fs from "node:fs/promises";

const state = {
  status: "ready",
  snapshot: {
    quotaGroups: [
      {
        id: "codex",
        name: "Codex",
        primary: true,
        planType: "ChatGPT",
        windows: [
          {
            key: "primary",
            label: "5-hour allowance",
            usedPercent: 28,
            remainingPercent: 72,
            windowDurationMins: 300,
            resetsAt: Math.floor(Date.now() / 1000) + 7_200,
          },
          {
            key: "weekly",
            label: "Weekly allowance",
            usedPercent: 43,
            remainingPercent: 57,
            windowDurationMins: 10_080,
            resetsAt: Math.floor(Date.now() / 1000) + 345_600,
          },
        ],
      },
    ],
    tokenActivity: { todayTokens: "415000", lifetimeTokens: "18450000", peakDailyTokens: "1270000" },
    credits: { hasCredits: true, unlimited: false, balance: "12.50", spendControlReached: false, individualLimit: null },
    updatedAt: Math.floor(Date.now() / 1000),
    stale: false,
  },
  message: null,
  codexVersion: "0.1.0",
  codexPath: "C:\\Users\\Example\\AppData\\Local\\Codex\\codex.exe",
  autostartEnabled: true,
  expanded: false,
  updating: false,
  refreshIntervalSecs: 60,
  checkpointPercentages: [50, 20, 10],
  checkpointNotification: null,
};

test.beforeAll(async () => {
  await fs.mkdir("artifacts/playwright", { recursive: true });
});

test.describe("overlay appearance", () => {
  for (const scenario of [
    { mode: "dark", background: "#090d18" },
    { mode: "light", background: "#f5f7fb" },
  ] as const) {
    test(`${scenario.mode} host background keeps the popup readable`, async ({ page }) => {
      await page.addInitScript(({ appState, theme }) => {
        window.localStorage.setItem("codex-usage-overlay-theme", theme);
        window.localStorage.setItem("codex-usage-overlay-palette", "heavymask");
        let callbackId = 0;
        const callbacks = new Map<number, (event: unknown) => void>();
        (window as Window & {
          __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => void };
        }).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
        const tauriWindow = window as Window & {
          __TAURI_INTERNALS__: {
            metadata: { currentWindow: { label: string } };
            transformCallback: (callback: (event: unknown) => void) => number;
            unregisterCallback: () => void;
            invoke: (command: string, args?: { expanded?: boolean }) => Promise<unknown>;
          };
        };
        tauriWindow.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: (callback) => {
            const id = ++callbackId;
            callbacks.set(id, callback);
            return id;
          },
          unregisterCallback: () => undefined,
          invoke: async (command, args) => {
            if (command === "get_app_state") return appState;
            if (command === "plugin:event|listen") return 1;
            if (command === "set_overlay_expanded") {
              appState.expanded = Boolean(args?.expanded);
              callbacks.forEach((callback) =>
                callback({ event: "usage-state-changed", id: 1, payload: appState }),
              );
            }
            return null;
          },
        };
      }, { appState: state, theme: scenario.mode });

      await page.goto("/");
      await page.evaluate((background) => {
        document.documentElement.style.background = background;
        document.body.style.background = background;
      }, scenario.background);

      const shell = page.locator(".overlay-shell");
      await expect(shell).toHaveClass(new RegExp(`theme-${scenario.mode}`));
      await expect(page.getByRole("heading", { name: "Codex Tracker" })).toBeVisible();
      await expect(page.locator(".hm-mark img")).toHaveAttribute("src", /heavymask-sammy/);
      await expect(page.getByText("Made by")).toHaveCount(0);

      await page.getByRole("button", { name: "Show token activity and settings" }).click();
      await expect(page.getByRole("region", { name: "Extended settings" })).toBeVisible();
      await expect(page.getByText("Made by")).toBeVisible();
      await page.getByRole("button", { name: "Open HeavyMask website" }).click();
      await expect(page.getByRole("button", { name: "Open HeavyMask website" })).toBeVisible();

      const themeBarBackgrounds = new Set<string>();
      const themePercentageText = new Set<string>();
      for (const theme of ["HeavyMask", "Ocean", "Orchid", "Ember", "Forest"]) {
        await page.getByRole("button", { name: `Select ${theme} theme` }).click();
        await expect(shell).toHaveClass(new RegExp(`palette-${theme.toLowerCase()}`));
        const barStyle = await page.locator(".progress-fill").first().evaluate((element) => {
          const computed = getComputedStyle(element);
          return { backgroundImage: computed.backgroundImage, color: computed.color };
        });
        expect(barStyle.backgroundImage).toContain("linear-gradient");
        themeBarBackgrounds.add(`${barStyle.backgroundImage}|${barStyle.color}`);
        const percentageTextStyle = await page.locator(".meter-heading strong").first().evaluate((element) => {
          const computed = getComputedStyle(element);
          return `${computed.backgroundImage}|${computed.webkitTextFillColor}`;
        });
        expect(percentageTextStyle).toContain("linear-gradient");
        themePercentageText.add(percentageTextStyle);
        await page.screenshot({
          path: `artifacts/playwright/overlay-${scenario.mode}-${theme.toLowerCase()}.png`,
        });

        if (theme === "HeavyMask") {
          const heavyMaskColors = await shell.evaluate((element) => {
            const computed = getComputedStyle(element);
            return [
              computed.getPropertyValue("--accent-light").trim(),
              computed.getPropertyValue("--accent-alt-light").trim(),
              computed.getPropertyValue("--accent-dark").trim(),
              computed.getPropertyValue("--accent-alt-dark").trim(),
            ];
          });
          expect(heavyMaskColors[0]).toBe(heavyMaskColors[1]);
          expect(heavyMaskColors[2]).toBe(heavyMaskColors[3]);
        }
      }
      expect(themeBarBackgrounds.size).toBe(5);
      expect(themePercentageText.size).toBe(5);

      await page.getByRole("button", { name: "Select HeavyMask theme" }).click();
      await expect(shell).toHaveClass(/palette-heavymask/);

      const bounds = await page.locator(".overlay-card").boundingBox();
      expect(bounds).not.toBeNull();
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.y).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(300);
      expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(520);
      expect(await page.locator(".content").evaluate((element) => (element as HTMLElement).scrollWidth)).toBeLessThanOrEqual(
        await page.locator(".content").evaluate((element) => (element as HTMLElement).clientWidth),
      );
      expect(await page.locator(".content").evaluate((element) => (element as HTMLElement).scrollHeight)).toBeGreaterThanOrEqual(
        await page.locator(".content").evaluate((element) => (element as HTMLElement).clientHeight),
      );

      await page.screenshot({ path: `artifacts/playwright/overlay-${scenario.mode}.png` });
    });
  }
});

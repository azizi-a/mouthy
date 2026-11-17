import { test, expect, type Page } from "@playwright/test";

// Just enough of Tauri's IPC for the Settings window to render outside the app.
async function mockTauri(page: Page) {
  await page.addInitScript(() => {
    let id = 0;
    const settings = { onboarding_completed: true, bindings: {} };
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "linux" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { windowLabel: "main", label: "main" },
        },
        transformCallback: () => ++id,
        unregisterCallback: () => {},
        invoke: async (cmd: string) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings") {
            return settings;
          }
          return cmd === "plugin:event|listen" ? ++id : null;
        },
      },
    });
  });
}

// The start-up probe in index.html shows its panel 5 s after the page starts loading.
test.describe("Start-up probe", () => {
  test("stays hidden on a healthy start", async ({ page }) => {
    await mockTauri(page);
    await page.goto("/");
    await expect(page.locator("#root")).toContainText("General");
    await page.waitForTimeout(6000);
    await expect(page.locator("body > pre")).toHaveCount(0);
  });

  test("explains a window whose script never loads", async ({ page }) => {
    await mockTauri(page);
    await page.route(/\/src\/main\.tsx(\?.*)?$/, (route) => route.abort());
    await page.goto("/");
    await expect(page.locator("body > pre")).toContainText(
      "has not started after 5 seconds",
      { timeout: 8000 },
    );
    await expect(page.locator("body > pre")).toContainText(
      "Script started: no",
    );
    await expect(page.locator("body > pre")).toContainText(
      "main.tsx (or one of its imports)",
    );
  });
});

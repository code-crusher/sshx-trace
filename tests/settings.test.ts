import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

const localStorageDescriptor = Object.getOwnPropertyDescriptor(
  window,
  "localStorage",
);

/** Restore the real jsdom `localStorage` after a test overrides it. */
function restoreLocalStorage() {
  if (localStorageDescriptor) {
    Object.defineProperty(window, "localStorage", localStorageDescriptor);
  }
}

/** Make `window.localStorage` throw, as it does when storage is blocked. */
function denyLocalStorageAccess() {
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    get() {
      throw new DOMException(
        "Failed to read the 'localStorage' property from 'Window': " +
          "Access is denied for this document.",
        "SecurityError",
      );
    },
  });
}

/** Import a fresh copy of the settings module, as happens on page load. */
async function loadSettings() {
  vi.resetModules();
  return await import("../src/lib/settings");
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  restoreLocalStorage();
  localStorage.clear();
});

describe("settings", () => {
  it("loads persisted settings when localStorage is available", async () => {
    localStorage.setItem(
      "sshx-settings-store",
      JSON.stringify({ name: "Ada", theme: "Dracula", scrollback: 100 }),
    );

    const { settings } = await loadSettings();

    expect(get(settings)).toEqual({
      name: "Ada",
      theme: "Dracula",
      scrollback: 100,
    });
  });

  it("persists updated settings to localStorage", async () => {
    const { settings, updateSettings } = await loadSettings();

    updateSettings({ name: "Grace" });

    expect(get(settings).name).toBe("Grace");
    expect(JSON.parse(localStorage.getItem("sshx-settings-store")!)).toEqual({
      name: "Grace",
    });
  });

  it("does not throw when localStorage access is denied", async () => {
    denyLocalStorageAccess();

    // This is the regression from issue #152: importing the module used to
    // throw a SecurityError and crash app initialization.
    const { settings } = await loadSettings();

    expect(get(settings)).toEqual({
      name: "",
      theme: "VS Code Dark",
      scrollback: 5000,
    });
  });

  it("keeps settings in memory when localStorage access is denied", async () => {
    denyLocalStorageAccess();

    const { settings, updateSettings } = await loadSettings();
    updateSettings({ name: "Ada", scrollback: 1000 });

    expect(get(settings)).toMatchObject({ name: "Ada", scrollback: 1000 });
  });

  it("falls back to memory when writing to localStorage fails", async () => {
    // Some browsers expose localStorage but reject writes, e.g. in private
    // browsing mode or when the storage quota is exhausted.
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: () => null,
        setItem: () => {
          throw new DOMException("Quota exceeded", "QuotaExceededError");
        },
        removeItem: () => {},
      },
    });

    const { settings, updateSettings } = await loadSettings();
    updateSettings({ name: "Ada" });

    expect(get(settings).name).toBe("Ada");
  });
});

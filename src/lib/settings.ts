import { persisted } from "svelte-persisted-store";
import themes, { type ThemeName, defaultTheme } from "./ui/themes";
import { derived, writable, type Readable, type Writable } from "svelte/store";

export type Settings = {
  name: string;
  theme: ThemeName;
  scrollback: number;
};

const STORAGE_KEY = "sshx-settings-store";

/**
 * Check whether `localStorage` can be used in the current context.
 *
 * Accessing `window.localStorage` throws a `SecurityError` when third-party
 * storage is blocked, such as when the app is embedded in a cross-origin
 * iframe. Browsers may also reject writes in private mode, so probe with a
 * write and read rather than just reading the property.
 */
function storageAvailable(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  try {
    const probe = "__sshx_storage_probe__";
    window.localStorage.setItem(probe, probe);
    window.localStorage.removeItem(probe);
    return true;
  } catch {
    return false;
  }
}

/**
 * Persisted settings, or an in-memory fallback when storage is unavailable.
 *
 * Without storage, settings are no longer remembered across reloads, but the
 * app still starts and remains fully usable.
 */
const storedSettings: Writable<Partial<Settings>> = storageAvailable()
  ? persisted<Partial<Settings>>(STORAGE_KEY, {})
  : writable<Partial<Settings>>({});

/** A persisted store for settings of the current user. */
export const settings: Readable<Settings> = derived(
  storedSettings,
  ($storedSettings) => {
    // Do some validation on all of the stored settings.
    const name = $storedSettings.name ?? "";

    let theme = $storedSettings.theme;
    if (!theme || !Object.hasOwn(themes, theme)) {
      theme = defaultTheme;
    }

    let scrollback = $storedSettings.scrollback;
    if (typeof scrollback !== "number" || scrollback < 0) {
      scrollback = 5000;
    }

    return {
      name,
      theme,
      scrollback,
    };
  },
);

export function updateSettings(values: Partial<Settings>) {
  storedSettings.update((settings) => ({ ...settings, ...values }));
}

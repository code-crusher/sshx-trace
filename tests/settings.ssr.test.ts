// @vitest-environment node
import { describe, expect, it } from "vitest";
import { get } from "svelte/store";

describe("settings (server-side rendering)", () => {
  it("imports and uses defaults when there is no window", async () => {
    expect(typeof window).toBe("undefined");

    const { settings } = await import("../src/lib/settings");

    expect(get(settings)).toEqual({
      name: "",
      theme: "VS Code Dark",
      scrollback: 5000,
    });
  });
});

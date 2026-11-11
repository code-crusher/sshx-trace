import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.ts"],
    // Inline this package so `vi.resetModules()` also resets its module-level
    // store cache between tests.
    server: {
      deps: {
        inline: ["svelte-persisted-store"],
      },
    },
  },
});

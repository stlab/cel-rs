import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 30_000,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:3419/project/docs/adam-slides/",
    viewport: { width: 1280, height: 720 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node tests/build-fixture.mjs && node tests/serve.mjs",
    env: {
      SLIDES_INPUT_DIR: process.env.SLIDES_DIR ?? "dist",
      SLIDES_DIR: "test-results/site",
    },
    url: "http://127.0.0.1:3419/project/docs/adam-slides/",
    reuseExistingServer: false,
    timeout: 30_000,
  },
});

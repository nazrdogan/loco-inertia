import { defineConfig } from "@playwright/test";

// End-to-end checks of the adapter against the real Inertia client, in production mode
// (built assets, asset version from the manifest, Node SSR). Build first: `npm run build`.
// Uses the installed Google Chrome; running servers on the same ports are reused.
export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  use: { baseURL: "http://127.0.0.1:5150", channel: "chrome", trace: "retain-on-failure" },
  webServer: [
    { command: "node ssr/ssr.js", port: 13714, reuseExistingServer: true },
    {
      command: "cargo run --release --bin demo-cli -- start",
      cwd: "..",
      url: "http://127.0.0.1:5150/",
      reuseExistingServer: true,
      timeout: 300_000,
      env: {
        LOCO_ENV: "production",
        BINDING: "127.0.0.1",
        INERTIA_FLASH_SECRET: "e2e-only-secret-".repeat(5),
        INERTIA_SECURE_COOKIES: "false",
      },
    },
  ],
});

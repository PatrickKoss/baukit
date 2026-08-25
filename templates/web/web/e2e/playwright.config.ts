import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env['E2E_WEB_PORT'] ?? '4173');
const baseURL = `http://127.0.0.1:${String(port)}`;

/**
 * The gate is hermetic: Playwright builds and serves the app itself and the
 * specs stub every API path from `qa.config.ts`, so no backend, database, or
 * identity provider takes part. Products that need a real stack should add a
 * second project rather than weaken this one.
 */
export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  forbidOnly: Boolean(process.env['CI']),
  retries: process.env['CI'] === undefined ? 0 : 1,
  workers: process.env['E2E_WORKERS'] === undefined ? 2 : Number(process.env['E2E_WORKERS']),
  reporter: [['line'], ['html', { open: 'never' }]],
  // Query clients retry with backoff before surfacing a failure, so give
  // assertions more room than Playwright's five-second default.
  expect: { timeout: 20_000 },
  use: {
    baseURL,
    // The specs observe application fetches through page.route. A service
    // worker claiming the page would hide them.
    serviceWorkers: 'block',
    trace: 'on-first-retry',
    video: 'on-first-retry',
  },
  projects: [
    {
      name: 'desktop-chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } },
    },
    { name: 'mobile-chrome', use: { ...devices['Pixel 7'] } },
    { name: 'webkit-desktop', use: { ...devices['Desktop Safari'] } },
    { name: 'mobile-safari', use: { ...devices['iPhone 14'] } },
  ],
  webServer: {
    command: `pnpm exec vite preview --port ${String(port)} --strictPort`,
    url: baseURL,
    reuseExistingServer: process.env['CI'] === undefined,
    timeout: 120_000,
  },
});

import { defineConfig, devices } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const port = Number(process.env['E2E_WEB_PORT'] ?? '{{ context.web_preview_port }}');
const baseURL = `http://localhost:${String(port)}`;
const webRoot = fileURLToPath(new URL('..', import.meta.url));
const allPurposes = process.env['E2E_ALL_PURPOSES'] === '1';
const logicOnElsewhere = allPurposes ? [] : [/@logic\b/];
const webkitSkip = (process.env['E2E_WEBKIT_SKIP'] ?? '')
  .split(',')
  .filter((path) => path !== '')
  .map((path) => fileURLToPath(new URL(`../${path}`, import.meta.url)))
  .map((file) => new RegExp(`^${file.replace(/[.*+?^$()|[\]\\{}]/g, '\\$&')}$`));
const webkitProject = { grepInvert: logicOnElsewhere, testIgnore: webkitSkip };

/**
 * The gate is hermetic: Playwright builds and serves the app itself and the
 * specs stub every API path from `qa.config.ts`, so no backend, database, or
 * identity provider takes part. Products that need a real stack should add a
 * second project rather than weaken this one.
 */
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
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
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1440, height: 900 },
      },
    },
    { name: 'mobile-chrome', grepInvert: logicOnElsewhere, use: { ...devices['Pixel 7'] } },
    { name: 'webkit-desktop', ...webkitProject, use: { ...devices['Desktop Safari'] } },
    { name: 'mobile-safari', ...webkitProject, use: { ...devices['iPhone 14'] } },
  ],
  webServer: {
    command: `pnpm exec vite preview --host localhost --port ${String(port)} --strictPort`,
    cwd: webRoot,
    url: baseURL,
    reuseExistingServer: process.env['CI'] === undefined,
    timeout: 120_000,
  },
});

import { defineConfig, devices } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('..', import.meta.url));
// The realm ships with this port. Global setup registers any other port with
// the web client before the specs run.
const port = Number(process.env['E2E_WEB_PORT'] ?? '5173');
const baseURL = `http://localhost:${String(port)}`;

/**
 * Runs `e2e/stack` against the composed Keycloak. Start it with
 * `docker compose up -d --wait keycloak` from the product root first.
 */
export default defineConfig({
  testDir: './stack',
  globalSetup: './stack/global-setup.ts',
  fullyParallel: true,
  forbidOnly: Boolean(process.env['CI']),
  reporter: [['line']],
  expect: { timeout: 20_000 },
  use: { baseURL, trace: 'retain-on-failure' },
  projects: [{ name: 'desktop-chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: `pnpm exec vite --port ${String(port)} --strictPort`,
    cwd: webRoot,
    url: baseURL,
    reuseExistingServer: true,
    timeout: 120_000,
  },
});

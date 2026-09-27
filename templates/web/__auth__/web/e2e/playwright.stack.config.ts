import { defineConfig, devices } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('..', import.meta.url));
// The realm's web client only accepts redirects to this origin.
const baseURL = 'http://localhost:5173';

/**
 * Runs `e2e/stack` against the composed Keycloak. Start it with
 * `docker compose up -d --wait keycloak` from the product root first.
 */
export default defineConfig({
  testDir: './stack',
  fullyParallel: true,
  forbidOnly: Boolean(process.env['CI']),
  reporter: [['line']],
  expect: { timeout: 20_000 },
  use: { baseURL, trace: 'retain-on-failure' },
  projects: [{ name: 'desktop-chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'pnpm exec vite --port 5173 --strictPort',
    cwd: webRoot,
    url: baseURL,
    reuseExistingServer: true,
    timeout: 120_000,
  },
});

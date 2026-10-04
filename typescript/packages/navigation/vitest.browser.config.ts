import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';
export default defineConfig({
  resolve: { alias: [{ find: /^react-native$/, replacement: 'react-native-web' }] },
  test: {
    include: ['src/**/*.browser.test.tsx'],
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: 'chromium' }, { browser: 'webkit' }],
    },
  },
});

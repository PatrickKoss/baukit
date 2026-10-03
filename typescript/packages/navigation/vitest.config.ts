import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: { include: ['src/**/*.test.ts', 'src/web.test.tsx'], environment: 'jsdom' },
});

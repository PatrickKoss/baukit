import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.{ts,tsx}'],
      // Composition and presentation are covered by the browser gate in `e2e/`,
      // where they run against a real DOM. Measuring them here would only
      // reward jsdom render assertions that prove less than the qa specs do.
      exclude: [
        'src/main.tsx',
        'src/App.tsx',
        'src/route-state-view.tsx',
        'src/**/*.test.{ts,tsx}',
      ],
      // Conservative floors that the generated app clears. Raise them as the
      // product grows; never lower one to make a red build green.
      thresholds: {
        lines: 70,
        statements: 70,
        functions: 70,
        branches: 70,
      },
    },
  },
});

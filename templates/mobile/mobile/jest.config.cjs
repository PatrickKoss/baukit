module.exports = {
  preset: 'jest-expo',
  testMatch: ['<rootDir>/src/**/*.test.ts', '<rootDir>/src/**/*.test.tsx'],
  clearMocks: true,
  // Composition and generated constants are excluded: wiring a client or
  // emitting a token sheet has no branch a unit test could pin down, and the
  // assertions that would cover them only restate the module.
  collectCoverageFrom: [
    'src/**/*.{ts,tsx}',
    '!src/**/*.test.{ts,tsx}',
    '!src/analytics.ts',
    '!src/app-shell.tsx',
    '!src/api.ts',
    '!src/action-button.tsx',
    '!src/localization/i18n.ts',
    '!src/theme.ts',
    '!src/tokens.ts',
  ],
  coverageReporters: ['text', 'lcov'],
  // Conservative floors that the generated app clears. Raise them as the
  // product grows; never lower one to make a red build green.
  coverageThreshold: {
    global: { branches: 70, functions: 70, lines: 70, statements: 70 },
  },
  transformIgnorePatterns: [],
};

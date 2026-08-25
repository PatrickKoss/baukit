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
  moduleNameMapper: {
    '^@baukit/a11y-core$': '<rootDir>/node_modules/@baukit/a11y-core/dist/index.js',
    '^@baukit/analytics-core$': '<rootDir>/node_modules/@baukit/analytics-core/dist/index.js',
    '^@baukit/api-runtime$': '<rootDir>/node_modules/@baukit/api-runtime/dist/index.js',
    '^@baukit/data-contracts$': '<rootDir>/node_modules/@baukit/data-contracts/dist/index.js',
    '^@baukit/data-contracts-expo-sqlite$':
      '<rootDir>/node_modules/@baukit/data-contracts-expo-sqlite/dist/index.js',
    '^@baukit/localization-core$': '<rootDir>/node_modules/@baukit/localization-core/dist/index.js',
    '^@baukit/preferences-core$': '<rootDir>/node_modules/@baukit/preferences-core/dist/index.js',
    '^@baukit/ui-tokens$': '<rootDir>/node_modules/@baukit/ui-tokens/dist/index.js',
  },
  transformIgnorePatterns: [],
};

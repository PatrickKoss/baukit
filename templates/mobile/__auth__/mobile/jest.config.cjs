module.exports = {
  preset: 'jest-expo',
  testMatch: ['<rootDir>/src/**/*.test.ts', '<rootDir>/src/**/*.test.tsx'],
  clearMocks: true,
  moduleNameMapper: {
    '^@baukit/a11y-core$': '<rootDir>/node_modules/@baukit/a11y-core/dist/index.js',
    '^@baukit/analytics-core$': '<rootDir>/node_modules/@baukit/analytics-core/dist/index.js',
    '^@baukit/api-runtime$': '<rootDir>/node_modules/@baukit/api-runtime/dist/index.js',
    '^@baukit/auth-native$': '<rootDir>/node_modules/@baukit/auth-native/dist/index.js',
    '^@baukit/auth-native/expo$': '<rootDir>/node_modules/@baukit/auth-native/dist/expo.js',
    '^@baukit/data-contracts$': '<rootDir>/node_modules/@baukit/data-contracts/dist/index.js',
    '^@baukit/data-contracts-expo-sqlite$':
      '<rootDir>/node_modules/@baukit/data-contracts-expo-sqlite/dist/index.js',
    '^@baukit/localization-core$': '<rootDir>/node_modules/@baukit/localization-core/dist/index.js',
    '^@baukit/preferences-core$': '<rootDir>/node_modules/@baukit/preferences-core/dist/index.js',
    '^@baukit/ui-tokens$': '<rootDir>/node_modules/@baukit/ui-tokens/dist/index.js',
  },
  transformIgnorePatterns: [],
};

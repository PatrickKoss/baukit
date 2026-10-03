module.exports = {
  preset: '@react-native/jest-preset',
  testMatch: ['<rootDir>/src/native.test.tsx'],
  transform: {
    '^.+\\.[jt]sx?$': ['babel-jest', { presets: ['module:@react-native/babel-preset'] }],
  },
  transformIgnorePatterns: [],
  moduleNameMapper: {
    '^(\\.{1,2}/.*)\\.js$': '$1',
    '^@baukit/a11y-core$': '<rootDir>/../a11y-core/dist/index.js',
    '^@baukit/ui-tokens$': '<rootDir>/../ui-tokens/dist/index.js',
  },
};

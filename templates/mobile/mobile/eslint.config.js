import { react, scripts } from '@baukit/eslint-config';
import uiTokens from '@baukit/ui-tokens/eslint';
import { fixupPluginRules } from '@eslint/compat';
import { defineConfig } from 'eslint/config';
import reactNativeA11y from 'eslint-plugin-react-native-a11y';
import globals from 'globals';

const appFiles = ['**/*.{ts,tsx}'];

export default defineConfig(
  {
    ignores: ['node_modules/**', '.expo/**', 'coverage/**', 'dist/**'],
  },
  ...react({ files: appFiles }),
  {
    files: appFiles,
    rules: {
      'preserve-caught-error': 'error',
      '@typescript-eslint/no-dynamic-delete': 'error',
    },
  },
  {
    files: ['**/*.tsx'],
    plugins: {
      'react-native-a11y': fixupPluginRules(reactNativeA11y),
    },
    rules: reactNativeA11y.configs.all.rules,
  },
  {
    files: appFiles,
    plugins: { '@baukit/ui-tokens': uiTokens },
    rules: {
      '@baukit/ui-tokens/no-raw-color': [
        'error',
        { allowedFiles: ['src/tokens.ts', 'src/theme.ts'] },
      ],
    },
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
      parserOptions: {
        project: ['./tsconfig.json'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  ...scripts({ files: ['scripts/**/*.{js,mjs,cjs}'] }),
);

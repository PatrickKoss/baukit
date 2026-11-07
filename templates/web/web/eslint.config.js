import { base, react, scripts, testRelaxedRules } from '@baukit/eslint-config';
import uiTokens from '@baukit/ui-tokens/eslint';
import { defineConfig } from 'eslint/config';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';

const appFiles = ['src/**/*.{ts,tsx}', 'csp/**/*.ts', 'vite.config.ts'];
const e2eFiles = ['e2e/**/*.ts'];

export default defineConfig(
  {
    ignores: ['dist/**', 'node_modules/**', 'coverage/**'],
  },
  ...react({ files: appFiles }),
  { ...reactRefresh.configs.vite, files: ['src/**/*.tsx'] },
  {
    files: appFiles,
    plugins: { '@baukit/ui-tokens': uiTokens },
    rules: {
      '@baukit/ui-tokens/no-raw-color': [
        'error',
        { allowedFiles: ['src/tokens.css', 'src/styles.css'] },
      ],
    },
    languageOptions: {
      globals: globals.browser,
      parserOptions: {
        project: ['./tsconfig.json'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  ...base({ files: e2eFiles }),
  {
    files: e2eFiles,
    languageOptions: {
      globals: globals.node,
      parserOptions: {
        project: ['./e2e/tsconfig.json'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // Specs read the browser DOM inside page.evaluate callbacks, where the
      // page's own globals are in scope rather than Node's.
      'no-undef': 'off',
      // Playwright's page.evaluate returns values typed from the callback, and
      // config-driven loops branch on optional entries.
      '@typescript-eslint/no-unnecessary-condition': 'off',
    },
  },
  { files: ['e2e/**/*.spec.ts'], rules: testRelaxedRules },
  ...scripts({ files: ['scripts/**/*.mjs'] }),
);

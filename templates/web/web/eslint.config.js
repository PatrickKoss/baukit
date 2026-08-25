import uiTokens from '@baukit/ui-tokens/eslint';
import eslint from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: ['dist/**', 'node_modules/**', 'coverage/**'],
  },
  eslint.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  reactHooks.configs.flat.recommended,
  reactRefresh.configs.vite,
  {
    files: ['**/*.{ts,tsx}'],
    ignores: ['e2e/**'],
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
  {
    files: ['e2e/**/*.ts'],
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
);

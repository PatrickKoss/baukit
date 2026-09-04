import eslint from '@eslint/js';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: ['**/dist/**', '**/node_modules/**'],
  },
  eslint.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  {
    files: ['packages/*/src/**/*.ts'],
    languageOptions: {
      parserOptions: {
        project: [
          './packages/*/tsconfig.json',
          './packages/auth-node/tsconfig.test.json',
          './tsconfig.test.json',
        ],
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
);

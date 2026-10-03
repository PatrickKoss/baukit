import { defineConfig } from 'eslint/config';
import base from '../../eslint.config.js';
export default defineConfig(base, {
  files: ['src/**/*.{ts,tsx}'],
  languageOptions: {
    parserOptions: {
      project: ['./tsconfig.json', './tsconfig.test.json'],
      tsconfigRootDir: import.meta.dirname,
    },
  },
});

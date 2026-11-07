import { react, scripts } from '@baukit/eslint-config';
import { reactWeb } from '@baukit/eslint-config/react-web';
import { defineConfig } from 'eslint/config';

const sourceFiles = ['packages/*/src/**/*.{ts,tsx}', 'packages/*/type-tests/**/*.ts'];
const toolConfigFiles = ['vitest.config.ts', 'packages/*/vitest*.config.ts'];

export default defineConfig(
  {
    ignores: ['**/dist/**', '**/node_modules/**', '**/coverage/**', '**/.turbo/**'],
  },
  ...reactWeb({ files: sourceFiles, domFiles: ['packages/navigation/src/web.tsx'] }),
  {
    files: sourceFiles,
    languageOptions: {
      parserOptions: {
        project: [
          './packages/*/tsconfig.json',
          './packages/*/tsconfig.test.json',
          './packages/data-contracts-dexie/tsconfig.browser.json',
          './packages/sync-client/tsconfig.exports.json',
        ],
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  ...react({ files: toolConfigFiles }),
  {
    files: toolConfigFiles,
    languageOptions: {
      parserOptions: {
        projectService: { allowDefaultProject: toolConfigFiles },
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  ...scripts({
    files: ['scripts/**/*.mjs', 'packages/*/scripts/**/*.{mjs,cjs}', 'packages/*/*.{js,cjs}'],
  }),
);

import eslintComments from '@eslint-community/eslint-plugin-eslint-comments';
import eslint from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export const sourceFiles = ['**/*.{ts,tsx,mts,cts}'];
export const testFiles = ['**/*.test.{ts,tsx}', '**/__tests__/**'];
export const scriptFiles = ['**/*.{js,mjs,cjs}'];

const MAX_FUNCTION_LINES = 80;
const MAX_FILE_LINES = 500;
const MAX_DEPTH = 4;
const MAX_PARAMS = 4;
const MAX_COMPLEXITY = 15;

/** Rules Baukit adds on top of the typescript-eslint strict and stylistic presets. */
export const baseRules = {
  'max-lines-per-function': [
    'error',
    { max: MAX_FUNCTION_LINES, skipBlankLines: true, skipComments: true },
  ],
  'max-lines': ['error', { max: MAX_FILE_LINES, skipBlankLines: true, skipComments: true }],
  'max-depth': ['error', MAX_DEPTH],
  'max-params': ['error', MAX_PARAMS],
  complexity: ['error', MAX_COMPLEXITY],
  '@typescript-eslint/consistent-type-assertions': [
    'error',
    { assertionStyle: 'as', objectLiteralTypeAssertions: 'never' },
  ],
  '@typescript-eslint/strict-boolean-expressions': 'error',
  '@typescript-eslint/explicit-module-boundary-types': 'error',
  '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
};

/** Size limits that do not fit test suites, which group many cases in one `describe`. */
export const testRelaxedRules = {
  'max-lines-per-function': 'off',
  'max-lines': 'off',
  'max-params': 'off',
};

/** Every eslint-disable names a rule and gives a `-- reason`; stale directives fail. */
function directiveConfigs(files) {
  return [
    {
      files,
      plugins: { '@eslint-community/eslint-comments': eslintComments },
      linterOptions: { reportUnusedDisableDirectives: 'error' },
      rules: {
        '@eslint-community/eslint-comments/require-description': 'error',
        '@eslint-community/eslint-comments/no-unlimited-disable': 'error',
      },
    },
  ];
}

/**
 * Type-checked TypeScript rules for `files`. Consumers add their own
 * `languageOptions.parserOptions` (`projectService` or `project` plus `tsconfigRootDir`).
 */
export function base({ files = sourceFiles } = {}) {
  return [
    { ...eslint.configs.recommended, files },
    ...[...tseslint.configs.strictTypeChecked, ...tseslint.configs.stylisticTypeChecked].map(
      (config) => ({ ...config, files }),
    ),
    { files, rules: baseRules },
    ...directiveConfigs(files),
    { files: testFiles, rules: testRelaxedRules },
  ];
}

/** `base` plus the React hooks rules. */
export function react({ files = sourceFiles } = {}) {
  return [...base({ files }), { ...reactHooks.configs.flat.recommended, files }];
}

const scriptRules = {
  ...Object.fromEntries(
    Object.entries(baseRules).filter(([rule]) => !rule.startsWith('@typescript-eslint/')),
  ),
  'no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
};

/** Plain JavaScript build and test scripts: no type information, Node globals by default. */
export function scripts({ files = scriptFiles, globals: scriptGlobals = globals.node } = {}) {
  return [
    { ...eslint.configs.recommended, files },
    ...directiveConfigs(files),
    {
      files,
      languageOptions: { sourceType: 'module', globals: scriptGlobals },
      rules: scriptRules,
    },
  ];
}

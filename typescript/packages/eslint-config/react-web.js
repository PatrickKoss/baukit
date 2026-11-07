import jsxA11y from 'eslint-plugin-jsx-a11y';
import { react, sourceFiles } from './index.js';

/** `react` plus the jsx-a11y recommended rules for DOM JSX in `domFiles`. */
export function reactWeb({ files = sourceFiles, domFiles = ['**/*.tsx'] } = {}) {
  return [...react({ files }), { ...jsxA11y.flatConfigs.recommended, files: domFiles }];
}

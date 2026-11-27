# `@baukit/eslint-config`

Shared ESLint flat configs for Baukit packages and products. Each export is a function that
returns an array of config objects, so a consumer spreads it into `defineConfig` and adds its own
parser options.

```js
import { defineConfig } from 'eslint/config';
import { react, scripts } from '@baukit/eslint-config';

export default defineConfig(
  { ignores: ['dist/**'] },
  ...react({ files: ['src/**/*.{ts,tsx}'] }),
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
  },
  ...scripts({ files: ['scripts/**/*.mjs'] }),
);
```

## Exports

| Export                                                      | What it is                                                               |
| ----------------------------------------------------------- | ------------------------------------------------------------------------ |
| `base(options)`                                             | Type-checked TypeScript rules for `files`, default `sourceFiles`.        |
| `react(options)`                                            | `base` plus `eslint-plugin-react-hooks` recommended.                     |
| `scripts(options)`                                          | Plain JavaScript without type information. `globals` defaults to Node's. |
| `reactWeb(options)`, from `@baukit/eslint-config/react-web` | `eslint-plugin-jsx-a11y` recommended for `domFiles`.                     |
| `baseRules`, `testRelaxedRules`                             | The rule records `base` adds.                                            |
| `sourceFiles`, `testFiles`, `scriptFiles`                   | The default globs.                                                       |

- `base` is `@eslint/js` recommended, typescript-eslint `strictTypeChecked` and
  `stylisticTypeChecked`, and the Baukit limits: 80 lines per function, 500 lines per file,
  depth 4, 4 parameters, complexity 15, no object literal type assertions,
  `strict-boolean-expressions` and explicit module boundary types. Test files (`*.test.ts(x)`,
  `__tests__/`) drop the function, file and parameter limits.
- `react` adds `eslint-plugin-react-hooks` recommended.
- `reactWeb` from `@baukit/eslint-config/react-web` adds `eslint-plugin-jsx-a11y` recommended for
  `domFiles`. Install `eslint-plugin-jsx-a11y` next to this package to use it.
- `scripts` lints plain JavaScript without type information and with Node globals. It keeps the
  `baseRules` limits that are not typescript-eslint rules.
- `base` and `scripts` also require a rule name and a `-- reason` on every `eslint-disable`, and
  report unused disable directives as errors.

`baukit check lints` requires every `eslint.config.{js,mjs,ts}` in a product to import this package,
and `baukit check suppressions` rejects a rule set to `off` or `warn` outside it. See
[`lint/README.md`](../../../lint/README.md) and the [frontend guide](../../../docs/guide/frontend.md).

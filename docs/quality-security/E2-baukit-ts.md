# E2: baukit TypeScript/JavaScript code quality

Scope: `typescript/packages/*` (24 packages), `typescript/eslint.config.js`, `typescript/tsconfig.base.json`, `templates/web`, `templates/mobile`. Side note on `scripts/`.

The short version: the baseline is already strict. `strictTypeChecked` + `stylisticTypeChecked` and every strict tsconfig flag on the list are on, in the packages and in both templates. Library `src/` has zero `any`, zero non-null assertions, zero `@ts-ignore`, zero `console.*`, one `eslint-disable`. What's left is structural: long React components, high-complexity validators, hand-rolled duplicated type guards ending in `as unknown as`, React hooks in packages that no hooks linter sees, and tokens persisted in `localStorage`.

## Reproduction

All per-rule counts come from one ESLint run that adds measurement rules on top of each package's existing config (nothing installed, nothing written to the repo):

```bash
T=/home/patrick/.cache/baukit-orchestrator/quality-security/tmp
cd /home/patrick/projects/baukit/typescript
RULES='{"max-lines-per-function":["error",{"max":80,"skipBlankLines":true,"skipComments":true}],"complexity":["error",15],"max-depth":["error",4],"max-lines":["error",{"max":500,"skipBlankLines":true,"skipComments":true}],"no-magic-numbers":["error",{"ignore":[0,1,-1,2],"ignoreArrayIndexes":true,"ignoreDefaultValues":true,"enforceConst":false}],"max-params":["error",4],"@typescript-eslint/consistent-type-assertions":["error",{"assertionStyle":"never"}],"@typescript-eslint/explicit-module-boundary-types":"error","@typescript-eslint/prefer-readonly":"error","@typescript-eslint/switch-exhaustiveness-check":"error","@typescript-eslint/strict-boolean-expressions":"error","@typescript-eslint/no-floating-promises":"error","@typescript-eslint/require-array-sort-compare":"error","no-console":"error","no-param-reassign":"error","eqeqeq":"error","@typescript-eslint/no-non-null-assertion":"error","@typescript-eslint/promise-function-async":"error"}'
for d in packages/*/; do (cd $d && ../../node_modules/.bin/eslint src --no-warn-ignored --rule "$RULES" -f json -o $T/eslint-$(basename $d).json); done
python3 $T/agg.py $T                # per-rule counts, src vs tests
python3 $T/agg.py $T <rule> 300     # list src violations for one rule
```

`agg.py` classifies a file as test when its path matches `\.test\.|__tests__|/vitest\.ts$`. Counts below are "src" (non-test) unless stated.

Per-rule result (src / tests): promise-function-async 247/402, no-magic-numbers 231/647, consistent-type-assertions(never) 126/204, strict-boolean-expressions 31/0, complexity 24/0, max-lines-per-function 13/71, eqeqeq 10/0, max-lines 9/18, explicit-module-boundary-types 7/0, max-params 6/0, max-depth 5/0, no-param-reassign 2/1. Zero hits: no-floating-promises, switch-exhaustiveness-check, prefer-readonly, no-console, no-non-null-assertion.

Environment caveat: `purchases-core` and `purchases-revenuecat` have no `node_modules` in this checkout, so their imports resolve to error types and the existing config reports 104 `no-unsafe-*` errors in purchases-core alone (`cd typescript/packages/purchases-core && ../../node_modules/.bin/eslint src --max-warnings=0`). That is a stale install (`corepack pnpm --dir typescript install`), not a code smell; I excluded `no-unsafe-*` from the findings.

## Findings (ranked by impact)

### E2-01 Access and refresh tokens persisted in localStorage (auth-web). Severity: high

`BrowserOidcClient` writes the full token set, including `refreshToken`, to `localStorage` as JSON. Any XSS in any product web app can read a long-lived refresh token.

- `typescript/packages/auth-web/src/index.ts:428` `this.environment.localStorage.setItem(this.tokenStorageKey, JSON.stringify(tokens));`
- `typescript/packages/auth-web/src/index.ts:448` `const raw = this.environment.localStorage.getItem(this.tokenStorageKey);`
- `typescript/packages/auth-web/src/index.ts:105` `readonly refreshToken?: string;` (part of `StoredTokens`)
- `typescript/packages/auth-web/src/index.ts:655` `localStorage: window.localStorage,` (default environment)

Prevalence: 1 package, 9 lines mentioning `localStorage` in auth-web src (interface field :83, default :655, 7 call sites). Command: `cd typescript/packages && grep -c "localStorage" auth-web/src/index.ts`. The templates also use `localStorage` (45 lines across `templates/web`, `templates/mobile`, `templates/common`: `cd templates && grep -rEn localStorage web mobile common | wc -l`), but those are the profile registry and analytics ID, not tokens.

Prevention: no lint rule covers this. Custom check needed: `no-restricted-syntax` selector `CallExpression[callee.property.name='setItem'][callee.object.property.name='localStorage']` in `auth-*` packages and templates, with an allowlist for non-credential keys. The real fix is a design decision (in-memory access token, refresh via the silent iframe broker that already exists in `silent-token-broker.ts`, or a BFF cookie); flag for the security report.

### E2-02 React hooks in library packages are not linted by eslint-plugin-react-hooks. Severity: high

The templates enable `reactHooks.configs.flat.recommended` (`templates/web/web/eslint.config.js:16`, `templates/mobile/mobile/eslint.config.js:16`), but the `typescript/` workspace has no react-hooks plugin at all (`ls typescript/node_modules/.pnpm | grep ^eslint-plugin` prints nothing). 13 non-test src files use hooks (navigation, a11y-core, purchases-core, auth-native), and `typescript/packages/navigation/eslint.config.js` only extends the base config.

Measured by borrowing the plugin from a product checkout (temp config `/home/patrick/.cache/baukit-orchestrator/quality-security/tmp/hooks.config.mjs`, imports `/home/patrick/projects/schluesselbund/web/node_modules/eslint-plugin-react-hooks`):

```bash
cd typescript && bash -c 'F=$(grep -rlE "\buse(Effect|State|Callback|Memo|Ref|LayoutEffect|SyncExternalStore)\(" --include=*.ts --include=*.tsx packages/*/src | grep -v test); node_modules/.bin/eslint -c $T/hooks.config.mjs $F -f json -o $T/hooks.json'
```

5 violations:
- `typescript/packages/navigation/src/native.tsx:833` exhaustive-deps: `if (bar) props.onBarHeightChange?.(barHeight); }, [bar, barHeight, props.onBarHeightChange]);`
- `typescript/packages/navigation/src/web.tsx:417` exhaustive-deps (useLayoutEffect reads `props`)
- `typescript/packages/navigation/src/native.tsx:427` set-state-in-effect: `presented.current = false; setWebPresented(false);`
- `typescript/packages/navigation/src/native.tsx:1091` set-state-in-effect
- `typescript/packages/purchases-core/src/react.ts:100` set-state-in-effect: `setState({ status: 'loading' });`

Prevention: add `eslint-plugin-react-hooks` (7.x, same version the templates pin) to `typescript/package.json` devDependencies and `reactHooks.configs.flat.recommended` to `typescript/eslint.config.js` for `packages/*/src/**/*.{ts,tsx}`. Fix cost: 5.

### E2-03 Hand-rolled validators that end in `as unknown as`. Severity: med

Several packages validate `unknown` JSON field-by-field and then double-cast, so the validator and the type can drift silently.

- `typescript/packages/purchases-core/src/account-plan.ts:73` `return value as unknown as VerifiedEntitlement;`
- `typescript/packages/purchases-core/src/account-plan.ts:97` `return value as unknown as CreditBalanceSnapshot;`
- `typescript/packages/suite-client/src/entitlements.ts:160` `const result = value as unknown as TicketClaims;`
- `typescript/packages/ui-tokens/src/validation.ts:164` `return current as unknown as ThemeColor;`

Prevalence: 17 `as unknown as` in src, 28 in tests. Command: `cd typescript/packages && bash -c 'grep -rn "as unknown as" --include=*.ts --include=*.tsx */src | grep -vE "\.test\.|__tests__" | wc -l'`. Of the 17, 5 are `globalThis` probes in `analytics-core/src/client.ts:68,76,83,88` and `api-runtime/src/index.ts:595`, 4 are timer-handle casts in `sync-client/src/browser.ts:61,63` and `sync-client/src/expo.ts:23,25`. All `as` assertions (`consistent-type-assertions` with `assertionStyle: never`): 126 src, top files analytics-core/src/client.ts 12, ui-tokens/src/eslint.ts 9, sync-client/src/browser.ts 7.

Prevention: `no-restricted-syntax` with selector `TSAsExpression > TSAsExpression.expression[typeAnnotation.type='TSUnknownKeyword']` (message: use a type guard returning `value is T`). Fix cost 17. `assertionStyle: never` (126) is too noisy to adopt now.

### E2-04 `isRecord` type guard copied ten times. Severity: med

- `typescript/packages/api-runtime/src/index.ts:731` `function isRecord(value: unknown): value is Record<string, unknown> {`
- `typescript/packages/auth-node/src/cache.ts:349` same
- `typescript/packages/auth-node/src/device-flow.ts:757` same (second copy inside one package)
- `typescript/packages/auth-web/src/index.ts:644` `function isObject(value: unknown): value is Record<string, unknown> {`
- also `auth-native/src/index.ts:887`, `purchases-core/src/account-plan.ts:45`, `suite-client/src/entitlements.ts:64`, `suite-client/src/plan-catalog.ts:12`, `ui-tokens/src/compiler.ts:8`, `ui-tokens/src/validation.ts:11`

Prevalence: 10. Command: `cd typescript/packages && bash -c 'grep -rnE "^(export )?(const|function) (isRecord|isObject|record|isPlainObject)\b" --include=*.ts --include=*.tsx */src | grep -vE "\.test\.|__tests__" | wc -l'`. Same pattern for scalar guards (`text`, `count`, `integer`) in purchases-core and suite-client. Together with E2-03 this points at one shared guard module (or a schema library) in a leaf package.

Prevention: no ESLint rule detects cross-package duplication. Custom check needed: `no-restricted-syntax` on `FunctionDeclaration[id.name=/^(isRecord|isObject)$/]` outside the one package that exports it. `jscpd` would catch the general case but is not installed.

### E2-05 Very long functions and components. Severity: med

13 src functions exceed 80 lines (`max-lines-per-function` 80, skip blanks/comments), 71 in tests. The 13:

| Function | Location | Lines |
|---|---|---|
| createSyncConformanceTests | sync-client/src/conformance.ts:254 | 574 |
| Menu | navigation/src/native.tsx:351 | 268 |
| AppNavigation | navigation/src/native.tsx:798 | 268 |
| (render arrow) | navigation/src/native.tsx:856 | 189 |
| AppNavigation | navigation/src/web.tsx:393 | 187 |
| createApiFetch | api-runtime/src/index.ts:277 | 142 |
| (fetch arrow) | api-runtime/src/index.ts:284 | 135 |
| Menu | navigation/src/web.tsx:108 | 136 |
| createRouteFocusController | a11y-core/src/route-focus.ts:50 | 115 |
| Profile | navigation/src/web.tsx:287 | 91 |
| createSqliteMigrationConformanceTests | data-contracts-expo-sqlite/src/migration-conformance.ts:192 | 90 |
| (arrow) | navigation/src/web.tsx:477 | 88 |
| Target | navigation/src/native.tsx:222 | 81 |

Two of the 13 are exported conformance suites (test builders shipped in src), which are long by nature. The navigation components are the real problem: `native.tsx` is 1198 lines with two 268-line components.

Prevalence: 13. Command: `python3 $T/agg.py $T max-lines-per-function 30`.
Prevention: `max-lines-per-function: ["error", {max: 80, skipBlankLines: true, skipComments: true}]` for src (off for `*.test.*` and `*conformance*.ts`). Fix cost 13 (11 excluding conformance).

### E2-06 High cyclomatic complexity in validators and request code. Severity: med

24 src functions above complexity 15. Worst:
- `typescript/packages/api-runtime/src/index.ts:284` fetch arrow inside `createApiFetch`, complexity 44 (also 5-6 levels deep, see E2-08)
- `typescript/packages/suite-client/src/entitlements.ts:177` `verifyTicket`, 29
- `typescript/packages/navigation/src/native.tsx:222` `Target`, 27
- `typescript/packages/analytics-core/src/client.ts:160` constructor, 24
- `typescript/packages/integrations-client/src/connection-health.ts:87` `reduceConnectionState`, 22

Prevalence: 24. Command: `python3 $T/agg.py $T complexity 30`.
Prevention: `complexity: ["error", 15]`. Fix cost 24. Start at 20 (11 violations) if 15 is too much at once.

### E2-07 Oversized files. Severity: med

9 src files exceed 500 non-blank, non-comment lines: navigation/src/native.tsx (1182), sync-client/src/conformance.ts (799), auth-native/src/index.ts (792), auth-node/src/device-flow.ts (761), navigation/src/web.tsx (685), api-runtime/src/index.ts (621), auth-web/src/index.ts (596), data-contracts/src/identity.ts (562), analytics-core/src/client.ts (522). Three auth packages and api-runtime put most of their logic in `index.ts`, so the barrel is also the implementation.

Evidence: `typescript/packages/navigation/src/native.tsx:510` (max-lines report line), `typescript/packages/auth-native/src/index.ts:571`.
Prevalence: 9. Command: `python3 $T/agg.py $T max-lines 30`; raw sizes: `cd typescript/packages && find */src \( -name '*.ts' -o -name '*.tsx' \) | grep -vE '\.test\.|__tests__' | xargs wc -l | sort -rn | sed -n 2,11p`.
Prevention: `max-lines: ["error", {max: 500, skipBlankLines: true, skipComments: true}]`. Fix cost 9.

### E2-08 Deep nesting in api-runtime retry loop. Severity: low

- `typescript/packages/api-runtime/src/index.ts:385` nested 6 deep
- `typescript/packages/api-runtime/src/index.ts:376`, `:391` nested 5
- `typescript/packages/analytics-core/src/client.ts:483`, `typescript/packages/preferences-core/src/controller.ts:141` nested 5

Prevalence: 5. Command: `python3 $T/agg.py $T max-depth 30`.
Prevention: `max-depth: ["error", 4]`. Fix cost 5 (3 disappear with the E2-06 split of createApiFetch).

### E2-09 Nullable booleans and numbers used directly in conditions. Severity: low

- `typescript/packages/navigation/src/web.tsx:138` nullable boolean in conditional
- `typescript/packages/navigation/src/use-navigation-state.ts:16` nullable number in conditional (0 and NaN treated as absent)
- `typescript/packages/purchases-core/src/access.ts:57` nullable boolean in conditional

Prevalence: 31. Command: `python3 $T/agg.py $T @typescript-eslint/strict-boolean-expressions 40`.
Prevention: `@typescript-eslint/strict-boolean-expressions` (default options). Fix cost 31. The number case is the one that actually produces bugs; `{allowNullableBoolean: true}` would cut the cost if the team only wants that.

### E2-10 Exported React components without explicit return types. Severity: low

Public API of a library; inferred JSX return types leak into `.d.ts` and change silently.
- `typescript/packages/navigation/src/native.tsx:798` `AppNavigation`
- `typescript/packages/navigation/src/web.tsx:393` `AppNavigation`
- also `native.tsx:1076`, `web.tsx:636`, `navigation/src/use-navigation-state.ts:10`, `suite-client/src/navigation.ts:14,66`

Prevalence: 7. Command: `python3 $T/agg.py $T @typescript-eslint/explicit-module-boundary-types 30`.
Prevention: `@typescript-eslint/explicit-module-boundary-types: "error"`. Fix cost 7.

### E2-11 Lint scope skips scripts, type tests and configs. Severity: low

Every package's lint is `eslint src --max-warnings=0` (24/24: `grep -h '"lint"' typescript/packages/*/package.json | sort | uniq -c`), and the root config only wires type information for `packages/*/src/**/*.ts` (`typescript/eslint.config.js:13`). Not linted: `pwa-web/scripts/build-worker.mjs`, `auth-node/scripts/keycloak-conformance.mjs`, `suite-client/scripts/build-cjs.mjs`, `sync-client/type-tests/*.ts`, and the vitest configs.

Prevalence: 12 files under `*/scripts` and `*/type-tests`. Command: `cd typescript/packages && bash -c 'find */scripts */type-tests -name "*.*js" -o -name "*.ts" 2>/dev/null | wc -l'`.
Prevention: change lint to `eslint . --max-warnings=0`, add a `files: ['packages/*/scripts/**/*.mjs']` block with `tseslint.configs.disableTypeChecked` and `globals.node` (the mobile template already does exactly this, `templates/mobile/mobile/eslint.config.js:52-56`). Fix cost unknown until enabled.

### E2-12 Web template has no DOM accessibility lint, mobile does. Severity: low

- `templates/mobile/mobile/eslint.config.js:27-32` applies `reactNativeA11y.configs.all.rules` to `**/*.tsx`
- `templates/web/web/eslint.config.js:16-17` has only `reactHooks` and `reactRefresh`; no `eslint-plugin-jsx-a11y`
- `typescript/packages/navigation/src/web.tsx` (699 lines of DOM menus) has neither

Prevalence: 0 of 2 web lint configs (template, navigation package) enable jsx-a11y. Command: `grep -rn "jsx-a11y" templates/web typescript/package.json typescript/packages/*/package.json | wc -l` gives 0. Not measured further: the plugin is not installed anywhere I could borrow it from (`ls -d /home/patrick/projects/*/web/node_modules/eslint-plugin-jsx-a11y` finds nothing).
Prevention: `eslint-plugin-jsx-a11y` `flatConfigs.recommended` in the web template and for `navigation/src/web.tsx`. Fix cost unmeasured.

### E2-13 Thin test coverage in several packages. Severity: low

Source files vs test files (non-test `.ts/.tsx` in `src` vs `*.test.ts(x)` anywhere in the package):

| Package | src | tests |
|---|---|---|
| analytics-core | 6 | 2 |
| notifications-core | 10 | 3 |
| localization-core | 9 | 4 |
| purchases-core | 11 | 5 |
| ui-tokens | 9 | 4 |
| suite-client | 14 | 8 |

Evidence: `typescript/packages/analytics-core/src/client.ts:160` (24-branch constructor) sits in a package with 2 test files; `typescript/packages/notifications-core/src/scheduler.ts:167` (`replaceNow`, 5 params) in one with 3.
Command: `cd typescript/packages && for d in */; do d=${d%/}; s=$(find $d/src -name '*.ts' -o -name '*.tsx' | grep -vE '\.test\.|__tests__|\.d\.ts' | wc -l); t=$(find $d -path '*/node_modules' -prune -o \( -name '*.test.ts' -o -name '*.test.tsx' \) -print | wc -l); echo "$d $s $t"; done` (run in bash).
Prevention: file ratios are a weak signal; the real check is vitest `coverage.thresholds` per package (lines/branches) enforced in `test` or `check`. Custom check needed otherwise: fail when a src module has no test importing it.

### E2-14 Functions with five or six positional parameters. Severity: low

- `typescript/packages/auth-node/src/device-flow.ts:505` `postForm`, 6 params
- `typescript/packages/auth-node/src/device-flow.ts:531` `requestJson`, 6 params
- also `auth-web/src/silent-token-broker.ts:326`, `data-contracts/src/import-envelope.ts:179`, `data-contracts/src/limits.ts:238`, `notifications-core/src/scheduler.ts:167`

Prevalence: 6. Command: `python3 $T/agg.py $T max-params 30`.
Prevention: `max-params: ["error", 4]`. Fix cost 6.

### Measured and rejected

- `eqeqeq`: all 10 src hits are the `x == null` / `x != null` idiom (`navigation/src/native.tsx:824`, `preferences-core/src/store.ts:60`, `auth-web/src/clerk.ts:50`). Adopt only as `["error", "always", {null: "ignore"}]`, which gives 0.
- `no-magic-numbers`: 231 src hits, mostly protocol constants (`auth-web/src/index.ts:420` `* 1000`, `:477` `400`) and test vectors in `sync-client/src/conformance.ts` (36). Too noisy for a lint rule; leave to review.
- `@typescript-eslint/promise-function-async`: 247 src. Style churn with no bug yield since `no-floating-promises` and `no-misused-promises` (already on) report 0.
- `no-param-reassign`: 2, both in `sync-client/src/hlc.ts:168-169` on local clock arithmetic. Not worth a rule.
- Circular imports: 2 cycles, both type-only (`auth-web/src/oidc-helpers.ts:1` `import type ... from './index.js'`; `auth-web/src/silent-token-browser.ts:1` imports a type from `silent-token-broker.ts`). Erased at runtime under `verbatimModuleSyntax`. Script: `cd typescript/packages && python3 $T/cycles.py`. `import/no-cycle` is not installed and not needed now.
- Module-level mutable state: 0 top-level `let` in src (`grep -rnE "^let " ... */src` excluding tests); the 11 module-level `new Set(...)` are read-only constant tables.
- `dangerouslySetInnerHTML`: 0 in packages and templates.

## Already enforced

Packages (`typescript/eslint.config.js`) and both templates use `eslint.configs.recommended` + `tseslint.configs.strictTypeChecked` + `stylisticTypeChecked`, so these are on everywhere and currently clean in src:

- `no-explicit-any` (0 `any` in src), `no-non-null-assertion` (0), `ban-ts-comment` (0 `@ts-ignore`; 7 `@ts-expect-error` all in tests), `no-floating-promises` and `no-misused-promises` (0), `no-unsafe-*` family, `no-unnecessary-condition`, `no-unnecessary-type-assertion`, `restrict-template-expressions`, `no-deprecated` (1 justified disable at `a11y-core/src/use-overlay-a11y.ts:49`), `no-unused-vars` with `^_` args.
- `no-console`: not configured, but src has 0 `console.*` calls.
- tsconfig (`typescript/tsconfig.base.json`, `templates/web/web/tsconfig.json`, `templates/mobile/mobile/tsconfig.json`): `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noImplicitOverride`, `noImplicitReturns`, `noFallthroughCasesInSwitch`, `noPropertyAccessFromIndexSignature`, `noUnusedLocals`, `noUnusedParameters`, `isolatedModules`, `verbatimModuleSyntax`. All 24 package tsconfigs extend the base (`grep -L tsconfig.base typescript/packages/*/tsconfig.json` prints nothing). No point testing `noUncheckedIndexedAccess` separately; it is already on.
- Templates: `react-hooks` recommended, `react-refresh` (web), `react-native-a11y` all rules (mobile), `@baukit/ui-tokens/no-raw-color`, `preserve-caught-error`, `no-dynamic-delete` (mobile).
- Prettier 3.9.9 with defaults (no config file), `format:check` per package via Turbo.

Templates are not weaker than packages. They duplicate the config instead of sharing it, but the type-checked presets are identical; templates are actually ahead on react-hooks and RN a11y.

## Recommended config

`typescript/eslint.config.js`, appended after the existing blocks:

```js
import reactHooks from 'eslint-plugin-react-hooks'; // new devDependency, 7.1.1 like the templates

  { files: ['packages/*/src/**/*.{ts,tsx}'], ...reactHooks.configs.flat.recommended },   // E2-02: 5
  {
    files: ['packages/*/src/**/*.{ts,tsx}'],
    ignores: ['**/*.test.*', '**/__tests__/**', '**/*conformance*.ts', '**/vitest.ts'],
    rules: {
      'no-restricted-syntax': ['error', {
        selector: "TSAsExpression > TSAsExpression.expression[typeAnnotation.type='TSUnknownKeyword']",
        message: 'Narrow with a type guard instead of `as unknown as`.',
      }],                                                                                  // E2-03: 17
      'max-lines-per-function': ['error', { max: 80, skipBlankLines: true, skipComments: true }], // E2-05: 11
      complexity: ['error', 15],                                                           // E2-06: 24
      'max-lines': ['error', { max: 500, skipBlankLines: true, skipComments: true }],      // E2-07: 8 (conformance excluded)
      'max-depth': ['error', 4],                                                           // E2-08: 5
      'max-params': ['error', 4],                                                          // E2-14: 6
      '@typescript-eslint/explicit-module-boundary-types': 'error',                        // E2-10: 7
      '@typescript-eslint/strict-boolean-expressions': 'error',                            // E2-09: 31
      eqeqeq: ['error', 'always', { null: 'ignore' }],                                     // 0
      'no-console': 'error',                                                               // 0, locks in current state
      '@typescript-eslint/switch-exhaustiveness-check': 'error',                           // 0
    },
  },
  { files: ['packages/*/scripts/**/*.mjs'], ...tseslint.configs.disableTypeChecked,
    languageOptions: { globals: globals.node } },                                          // E2-11
```

Package `lint` scripts: `eslint . --max-warnings=0` instead of `eslint src` (E2-11). `navigation/eslint.config.js` inherits the above.

Templates (`templates/web/web/eslint.config.js`): add `eslint-plugin-jsx-a11y` `flatConfigs.recommended` (E2-12, cost unmeasured), plus the same `no-restricted-syntax`, `complexity`, `max-lines-per-function`, `eqeqeq` null-ignore and `explicit-module-boundary-types` off (apps do not need it). Ideally ship these as a shared `@baukit/eslint-config` so the 12 products and the packages stop drifting by copy.

tsconfig: no changes. Every flag on the checklist is already on in base and both templates.

Custom checks (no linter covers them): E2-01 token writes to Web Storage, E2-04 duplicated `isRecord` guards (`no-restricted-syntax` on the function name outside the owning package), E2-13 per-package vitest coverage thresholds.

Total fix cost for the recommended package rules: 5 + 17 + 11 + 24 + 8 + 5 + 6 + 7 + 31 = 114 violations, most of them in `navigation`, `api-runtime`, `suite-client` and `analytics-core`.

## Side note: scripts/

`scripts/` has 33 `.py` and 15 `.sh` files (`ls scripts/*.py | wc -l`, `ls scripts/*.sh | wc -l`). Neither ruff nor shellcheck runs anywhere: no hit in `.github/`, `Makefile` or `scripts/ci-gate.*` (`grep -rlnE "ruff|shellcheck|flake8|pylint|mypy" .github scripts Makefile` only matches `scripts/expo-android-conformance.sh`, which carries `# shellcheck` directives at lines 3 and 42 but is never checked). Neither tool is installed on this machine. Recommend `ruff check` + `ruff format --check` and `shellcheck scripts/*.sh` as a ci-gate step.

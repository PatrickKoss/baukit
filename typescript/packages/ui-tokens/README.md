# `@baukit/ui-tokens`

A dependency-free design-token schema, validator, accessibility checker, and deterministic compiler for web and React Native.

## Define and validate tokens

```ts
import { checkContrast, parseTokens, type DesignTokens } from '@baukit/ui-tokens';

const tokens = parseTokens(untrustedJson);
const violations = checkContrast(tokens);
```

Colors are opaque `#RGB` or `#RRGGBB` sRGB values with light and dark variants. Color leaves require at least a semantic group and name. Typography, space, radius, motion, and elevation are named scales. `contrastPairs` declares foreground/background semantic paths; normal text must reach 4.5:1 and entries marked `largeText` must reach 3:1 in both themes. Validation errors include paths such as `$.color.text.primary.light`, while contrast failures include the calculated ratio.

Validation is handwritten rather than Zod-based: the schema is small, this keeps production dependencies at zero, and it enables diagnostics tailored to semantic token paths.

## Work with colors

The color helpers accept three- or six-digit hexadecimal sRGB colors, with an
optional `#`. Normalized and calculated colors use lowercase `#rrggbb` form.

```ts
import {
  blendColors,
  chooseReadableForeground,
  hexToRgb,
  normalizeHexColor,
  rgbToHex,
} from '@baukit/ui-tokens';

normalizeHexColor('09C'); // '#0099cc'
hexToRgb('#0099cc'); // { r: 0, g: 153, b: 204 }
rgbToHex({ r: 0, g: 153, b: 204 }); // '#0099cc'
blendColors('#ffffff', '#000000', 0.25); // '#404040'

const foreground = chooseReadableForeground('#005fcc', ['#ffffff', '#111111'], 4.5);
// { foreground: '#ffffff', ratio: 5.98..., meetsThreshold: true }
```

`blendColors` clamps its ratio to `[0, 1]`. `chooseReadableForeground` returns
the first candidate that meets the threshold. If none passes, it returns the
candidate with the highest ratio and sets `meetsThreshold` to `false`.

## Check a semantic contrast matrix

`DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS` defines these role paths:

- `color.background.primary` and `color.background.elevated`
- `color.text.primary` and `color.text.muted`
- `color.border.primary` and `color.focus.ring`
- `color.action.primary`, `color.action.onPrimary`, `color.action.secondary`, and `color.action.onSecondary`
- `color.status.success`, `color.status.warning`, `color.status.danger`, and their `onSuccess`, `onWarning`, and `onDanger` foregrounds

Text and labeled foregrounds require 4.5:1. Borders and focus rings require
3:1 against both background roles. The checker tests every requirement in the
light and dark variants and returns every failure.

```ts
import {
  checkSemanticContrastMatrix,
  DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS,
} from '@baukit/ui-tokens';

const failures = checkSemanticContrastMatrix(tokens, [
  ...DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS,
  {
    foregroundRole: 'color.chart.label',
    backgroundRole: 'color.chart.background',
    minimumRatio: 4.5,
  },
]);
```

Pass a different list to replace the defaults. A missing role or a minimum
outside the possible WCAG range of 1 through 21 throws a descriptive error.

## Calculate vertical layout

Products supply every dimension and threshold. Both functions reject negative
or non-finite values.

```ts
import { getUsableContentHeight, isShortViewport } from '@baukit/ui-tokens';

getUsableContentHeight(720, 112, 128); // 480
isShortViewport(600, productLayout.shortViewportHeight); // true
```

## Compile

```ts
import { toCssVariables, toReactNative } from '@baukit/ui-tokens';

const css = toCssVariables(tokens);
const nativeModule = toReactNative(tokens);
```

`toCssVariables` emits sorted `--bk-*` declarations in `:root`, then dark color overrides under `[data-theme="dark"]`. `toReactNative` emits a sorted nested `tokens` constant with an inferred `Tokens` type. Accessibility declarations are build metadata and are omitted from generated runtime constants.

## Enforce tokens with `no-raw-color`

The design-token schema is only worth having if colors stay out of components.
`@baukit/ui-tokens/eslint` ships an ESLint flat-config plugin that reports CSS
colors written directly in application code.

```js
import uiTokens from '@baukit/ui-tokens/eslint';

export default [
  {
    plugins: { '@baukit/ui-tokens': uiTokens },
    rules: {
      '@baukit/ui-tokens/no-raw-color': [
        'error',
        { allowedFiles: ['src/theme/tokens.ts', 'src/theme/**'] },
      ],
    },
  },
];
```

`configs.recommended` turns the rule on with defaults when a product needs no
options:

```js
import { configs } from '@baukit/ui-tokens/eslint';

export default [configs.recommended];
```

The rule reports `#rgb`, `#rrggbb`, and `#rrggbbaa` hex values plus
`rgb()`, `rgba()`, `hsl()`, and `hsla()` anywhere they appear, in string and
template literals alike. CSS color keywords such as `crimson` are reported only
in a style-like position: a color-named property, a `StyleSheet.create` call, a
`style`, `color`, or `fill` JSX attribute, a `css` tagged template, or a
variable whose name ends in `style`, `theme`, `palette`, or `colors`. That
restriction keeps route names, test ids, and ordinary prose off the report.

Options:

| Option                      | Default                                          | Meaning                                                                                                                              |
| --------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| `allowedFiles`              | `[]`                                             | Glob-like paths that may declare raw colors. `*` matches inside a path segment, `**` across segments. Put the token definition here. |
| `allowedValues`             | `transparent`, `currentColor`, `inherit`, `none` | Literals allowed everywhere. Entries add to the defaults.                                                                            |
| `additionalStyleProperties` | `[]`                                             | Extra property or attribute names treated as style-like.                                                                             |
| `reportKeywords`            | `true`                                           | Set to `false` to report only hex and functional notations.                                                                          |

## Boundaries

This package shares semantic values such as `color.background.primary`. It is deliberately not a
design system and not a cross-platform component library: DOM and React Native components,
interaction behavior, raw brand-palette decisions, and product themes all stay outside it.

`exampleTokens` demonstrates every group as a test fixture. It is not a Baukit visual language, and
copying it into a product as a starting palette is not what it is for.

The production entry point has no runtime dependencies. `eslint` is an optional peer dependency
reached only through the `/eslint` subpath, so importing the package never loads it.

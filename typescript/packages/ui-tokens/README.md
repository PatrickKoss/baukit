# `@baukit/ui-tokens`

A dependency-free design-token schema, validator, accessibility checker, and deterministic compiler for web and React Native.

This package shares semantic values such as `color.background.primary`; it is deliberately not a design system or cross-platform component library. DOM/React Native components, interaction behavior, raw brand-palette decisions, and product-specific themes remain outside the package.

## Define and validate tokens

```ts
import { checkContrast, parseTokens, type DesignTokens } from '@baukit/ui-tokens';

const tokens = parseTokens(untrustedJson);
const violations = checkContrast(tokens);
```

Colors are opaque `#RGB` or `#RRGGBB` sRGB values with light and dark variants. Color leaves require at least a semantic group and name. Typography, space, radius, motion, and elevation are named scales. `contrastPairs` declares foreground/background semantic paths; normal text must reach 4.5:1 and entries marked `largeText` must reach 3:1 in both themes. Validation errors include paths such as `$.color.text.primary.light`, while contrast failures include the calculated ratio.

Validation is handwritten rather than Zod-based: the schema is small, this keeps production dependencies at zero, and it enables diagnostics tailored to semantic token paths.

## Compile

```ts
import { toCssVariables, toReactNative } from '@baukit/ui-tokens';

const css = toCssVariables(tokens);
const nativeModule = toReactNative(tokens);
```

`toCssVariables` emits sorted `--bk-*` declarations in `:root`, then dark color overrides under `[data-theme="dark"]`. `toReactNative` emits a sorted nested `tokens` constant with an inferred `Tokens` type. Accessibility declarations are build metadata and are omitted from generated runtime constants. `exampleTokens` is available as a test fixture demonstrating every group; it is intentionally not a Baukit visual language.

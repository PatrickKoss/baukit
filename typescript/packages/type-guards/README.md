# `@baukit/type-guards`

Runtime type guards for validating `unknown` JSON without double casts. The package has no
dependencies. The Baukit packages that parse tokens, plans and API payloads share it.

## Exports

| Guard                      | Accepts                                           |
| -------------------------- | ------------------------------------------------- |
| `isRecord`                 | any object that is not `null` and not an array    |
| `isString`                 | any string                                        |
| `isNonEmptyString`         | a string with at least one character              |
| `isBoolean`                | `true` or `false`                                 |
| `isSafeInteger`            | a number for which `Number.isSafeInteger` is true |
| `isNonNegativeSafeInteger` | a safe integer of zero or more                    |
| `isStringArray`            | an array whose every item is a string             |
| `nullable(guard)`          | whatever `guard` accepts, and `null`              |

Each guard is a TypeScript type predicate, so a passing check narrows the value.

## Example

```ts
import { isNonEmptyString, isRecord, isSafeInteger, nullable } from '@baukit/type-guards';

interface Claims {
  readonly sub: string;
  readonly exp: number | null;
}

const isOptionalInteger = nullable(isSafeInteger);

export function parseClaims(value: unknown): Claims | undefined {
  if (!isRecord(value)) return undefined;
  const { sub, exp } = value;
  if (!isNonEmptyString(sub) || !isOptionalInteger(exp)) return undefined;
  return { sub, exp };
}
```

The baukit rules forbid `as unknown as T` on untrusted data. Use these guards instead. See the
[frontend guide](../../../docs/guide/frontend.md).

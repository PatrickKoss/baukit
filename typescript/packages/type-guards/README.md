# `@baukit/type-guards`

Runtime type guards for validating `unknown` JSON without double casts. The package has no
dependencies and is shared by the Baukit packages that parse tokens, plans and API payloads.

```ts
import { isNonEmptyString, isRecord } from '@baukit/type-guards';

function subject(claims: unknown): string | undefined {
  if (!isRecord(claims)) return undefined;
  return isNonEmptyString(claims['sub']) ? claims['sub'] : undefined;
}
```

`isRecord` accepts plain objects only, not `null` and not arrays. `nullable(guard)` extends any
guard with `null`.

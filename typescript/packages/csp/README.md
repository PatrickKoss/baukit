# `@baukit/csp`

The Content Security Policy that generated web and mobile web exports serve (ADR 0006 section 7).
Products keep only their origins; the directives live here, so every product gets the same policy.

```ts
import { contentSecurityPolicy, originOf, SECURITY_HEADERS } from '@baukit/csp';

const policy = contentSecurityPolicy({
  styles: 'files',
  origins: { api: originOf(process.env.VITE_API_URL) },
  scriptHashes: [],
});
```

`contentSecurityPolicy` throws on any script source that is not a `'sha256-...'` hash, and leaves
out `upgrade-insecure-requests` only when an origin is plain `http:`.
`developmentContentSecurityPolicy` adds inline styles and the HMR socket for the dev server.
`unhashedInlineScripts` lists inline scripts a built page carries that the policy does not hash.

`SECURITY_HEADERS` holds the headers sent next to the policy: COOP and CORP `same-origin` and
`nosniff`. COEP stays unset because `require-corp` blocks third-party images and analytics that do
not opt in.

# `@baukit/csp`

The Content Security Policy that generated web and mobile web exports serve, from ADR 0006
section 7. Products keep only their origins. The directives live here, so every product gets the
same policy.

## Exports

| Export                                                  | Does                                                                                                                |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `contentSecurityPolicy(options)`                        | Builds the production policy string.                                                                                |
| `developmentContentSecurityPolicy(options, hmrOrigins)` | The dev server variant, with inline styles and the HMR socket.                                                      |
| `assertProductionOrigins(origins)`                      | Throws for `http:` origins, embedded credentials and loopback hosts.                                                |
| `originOf(url)`                                         | The origin of a configured URL, or `undefined` when it is unset or blank. A value without a scheme gets `https://`. |
| `hashSource(digest)`                                    | Formats a base64 SHA-256 digest as `'sha256-...'`.                                                                  |
| `inlineScripts(html)`                                   | The text of every inline `<script>` in a page.                                                                      |
| `unhashedInlineScripts(html, policy, hash)`             | The inline scripts whose hash `script-src` lacks.                                                                   |
| `clerkFrontendApiOrigin(publishableKey)`                | Decodes the Clerk frontend API origin from a publishable key.                                                       |
| `SECURITY_HEADERS`                                      | The headers sent next to the policy.                                                                                |

`PolicyOptions` takes `styles` (`'files'` or `'runtime-injected'`), `origins`, `scriptHashes` and
an optional `blobFrames`. `PolicyOrigins` names `api`, `issuer`, `analytics`, `media` and
`suiteHub`, plus the lists `vendorConnect`, `vendorImages`, `vendorScripts` and `vendorFrames`.

## Example

```ts
import {
  assertProductionOrigins,
  contentSecurityPolicy,
  originOf,
  SECURITY_HEADERS,
} from '@baukit/csp';

const origins = {
  api: originOf('https://api.example.com'),
  issuer: originOf('https://identity.example.com'),
};
assertProductionOrigins(origins);

export const headers: Record<string, string> = {
  'Content-Security-Policy': contentSecurityPolicy({ styles: 'files', origins, scriptHashes: [] }),
  ...SECURITY_HEADERS,
};
```

`contentSecurityPolicy` throws on any script source that is not a `'sha256-...'` hash. It leaves out
`upgrade-insecure-requests` only when an origin is plain `http:`. `vendorScripts` adds origins to
`script-src`, `vendorFrames` adds them to `frame-src`, and `blobFrames: true` adds `blob:` to
`frame-src`. A production export calls `assertProductionOrigins`, so the build fails when a local
`.env` leaks into it.

`SECURITY_HEADERS` sets `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy` to
`same-origin`, and `X-Content-Type-Options` to `nosniff`. COEP stays unset because `require-corp`
blocks third-party images and analytics that do not opt in.

See the [frontend guide](../../../docs/guide/frontend.md) and
[ADR 0006](../../../docs/adr/0006-browser-token-handling.md).

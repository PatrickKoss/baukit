---
'@baukit/auth-node': minor
---

Add `parseApiOrigin(value, { allowLoopbackHttp?, label? })` and `ApiOriginError` to the package root. The parser trims the value, accepts one trailing slash, and returns the URL's origin. It rejects credentials, a path, a query, or a fragment, and it allows plain HTTP only on a loopback host when `allowLoopbackHttp` is set. Errors carry a `reason` of `invalid_url`, `insecure_scheme`, or `not_an_origin` and never include the value. The device flow now uses the same scheme and loopback check.

The generated MCP template now always depends on `@baukit/auth-node` and validates its API URL with `parseApiOrigin`.

No breaking changes to the package. The CLI doctor now requires `@baukit/auth-node` in every generated `mcp/package.json`.

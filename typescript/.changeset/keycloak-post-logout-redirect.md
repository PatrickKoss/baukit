---
'@baukit/auth-node': patch
---

`allowKeycloakWebOrigin(stack, origin)` also adds `<origin>/*` to the web client's `post.logout.redirect.uris` attribute, keeping the other attributes and any existing entries, so a signed-out test lands back on the app. A `+` entry already covers the redirect URIs and stays as it is. The `/keycloak-testing` subpath stays ESM only; the README now states that CommonJS consumers load it through Node 24 `require(esm)` and type-check it with `"module": "node20"` or `"nodenext"`, and the packed-exports test loads it with `require`.

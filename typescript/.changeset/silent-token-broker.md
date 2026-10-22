---
"@baukit/auth-web": minor
---

Add `SilentTokenBroker` for silent authorization against multiple OIDC realms with PKCE, verified account claims, in-memory token caches and explicit popup login. Blocked popups preserve earlier remembered failures; closed popups return `login_required` with reason `interaction_cancelled`. Custom environments can expose popup closure through optional `SilentAuthorizationWindow.closed`. Construction outside a browser without an explicit environment throws `SilentTokenError` with code `unsupported_environment`. Add `completeSilentTokenCallback` and the `CALLBACK_MESSAGE` constant for same-origin callback pages.

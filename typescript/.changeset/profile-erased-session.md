---
'@baukit/auth-web': minor
'@baukit/auth-native': minor
---

Add the `profile_erased` session-expired reason to `clearSession`. OIDC persists a force-login marker and requests `prompt=login` until a successful sign-in. Clerk and browser WorkOS emit the reason; their provider integrations need fresh authentication and JWT `auth_time` for re-onboarding.

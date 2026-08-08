# {{ context.app_name }} mobile

Expo/React Native application with product-local `expo-auth-session` authorization-code + PKCE authentication.

Run the composed Keycloak, copy `.env.example` to `.env`, then run `pnpm install && npx expo start`. Sign in with `test` / `password`. Tokens are kept in Expo SecureStore, refreshed before expiry, and cleared before Keycloak logout. Physical devices must use your computer's LAN address for both API and issuer URLs.

The auth client deliberately lives in `src/auth.ts`; it is not a shared package until two products prove the seam. Native compilation is outside fixture CI; run `pnpm typecheck`, `pnpm lint`, and `pnpm test` for the portable checks.

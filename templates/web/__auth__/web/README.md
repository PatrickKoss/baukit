# {{ context.app_name }} web

Vite, React, and TanStack Query application with product-local OIDC authorization-code + PKCE authentication.

Run `docker compose up -d keycloak` from the product root, copy `.env.example` to `.env`, then run `pnpm install && pnpm dev`. Sign in with `test` / `password`. The browser stores the short-lived token set locally, refreshes before expiry, and uses Keycloak's end-session endpoint for logout. No client secret is shipped to the browser.

The auth client deliberately lives in `src/auth.ts`; it is not a Baukit shared package until two real products prove the seam. The remaining Baukit packages come from {{ context.baukit_typescript_dependency_description }}.

Run `pnpm build`, `pnpm lint`, and `pnpm test` before shipping.

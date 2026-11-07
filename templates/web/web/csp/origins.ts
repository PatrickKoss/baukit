{% if context.auth_clerk %}import { clerkFrontendApiOrigin, originOf, type PolicyOrigins } from '@baukit/csp';
{% else %}import { originOf, type PolicyOrigins } from '@baukit/csp';
{% endif %}
export type BuildEnv = Readonly<Record<string, string | undefined>>;

function configured(env: BuildEnv, name: string): string | undefined {
  const value = env[name];
  return value === undefined || value.trim() === '' ? undefined : value;
}

/** The origins the app talks to, resolved the way `src/` resolves them at build time. */
{% if context.auth_enabled %}export function policyOrigins(env: BuildEnv, mode: string): PolicyOrigins {
{% else %}export function policyOrigins(env: BuildEnv): PolicyOrigins {
{% endif %}{% if context.auth_enabled %}  const production = mode === 'production';
{% endif %}  return {
{% if context.auth_enabled %}    api: originOf(
      configured(env, 'VITE_API_URL') ??
        (production ? undefined : 'http://localhost:{{ context.api_host_port }}'),
    ),
{% else %}    api: originOf(configured(env, 'VITE_API_URL') ?? 'http://localhost:{{ context.api_host_port }}'),
{% endif %}{% if context.auth_oidc %}    issuer: originOf(
      configured(env, 'VITE_OIDC_ISSUER') ?? 'http://localhost:{{ context.keycloak_host_port }}',
    ),
{% elif context.auth_enabled %}    // Production builds drop the local Keycloak adapter, so its issuer stays out.
    issuer: production
      ? undefined
      : originOf(
          configured(env, 'VITE_LOCAL_OIDC_ISSUER') ?? 'http://localhost:{{ context.keycloak_host_port }}',
        ),
{% endif %}    analytics: originOf(configured(env, 'VITE_POSTHOG_HOST')),
    media: originOf(configured(env, 'VITE_MEDIA_ORIGIN')),
    suiteHub: originOf(configured(env, 'VITE_SUITE_HUB_API_URL')),
{% if context.auth_clerk %}    vendorConnect: [clerkFrontendApiOrigin(configured(env, 'VITE_CLERK_PUBLISHABLE_KEY'))].filter(
      (origin): origin is string => origin !== undefined,
    ),
    vendorImages: ['https://img.clerk.com'],
{% elif context.auth_workos %}    vendorConnect: ['https://api.workos.com'],
{% endif %}  };
}

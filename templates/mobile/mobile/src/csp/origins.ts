{% if context.auth_clerk %}import { clerkFrontendApiOrigin, originOf, type PolicyOrigins } from '@baukit/csp';
{% else %}import { originOf, type PolicyOrigins } from '@baukit/csp';
{% endif %}
export type BuildEnv = Readonly<Record<string, string | undefined>>;

function configured(env: BuildEnv, name: string): string | undefined {
  const value = env[name];
  return value === undefined || value.trim() === '' ? undefined : value;
}

/** The origins the web export talks to. An unset API URL means the app's own origin. */
export function policyOrigins(env: BuildEnv): PolicyOrigins {
  return {
    api: originOf(configured(env, 'EXPO_PUBLIC_API_URL')),
{% if context.auth_oidc %}    issuer: originOf(configured(env, 'EXPO_PUBLIC_OIDC_ISSUER')),
{% endif %}    analytics: originOf(configured(env, 'EXPO_PUBLIC_POSTHOG_HOST')),
    media: originOf(configured(env, 'EXPO_PUBLIC_MEDIA_ORIGIN')),
    suiteHub: originOf(configured(env, 'EXPO_PUBLIC_SUITE_HUB_API_URL')),
{% if context.auth_clerk %}    vendorConnect: [clerkFrontendApiOrigin(configured(env, 'EXPO_PUBLIC_CLERK_PUBLISHABLE_KEY'))].filter(
      (origin): origin is string => origin !== undefined,
    ),
    vendorImages: ['https://img.clerk.com'],
{% elif context.auth_workos %}    vendorConnect: [
      originOf(configured(env, 'EXPO_PUBLIC_WORKOS_API_HOSTNAME') ?? 'api.workos.com'),
    ].filter((origin): origin is string => origin !== undefined),
{% endif %}  };
}

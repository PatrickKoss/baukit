import type { ConfigContext, ExpoConfig } from 'expo/config';

import { type BuildVariant, buildVariantFrom, withBuildVariant } from './src/build-variant.ts';
import { {% if context.auth_oidc %}MOBILE_OIDC_CLIENT_ID, {% elif context.auth_workos %}OIDC_REDIRECT_PATH, {% endif %}PRODUCT_NAME } from './src/product.ts';

const configuredApiUrl: unknown = process.env['EXPO_PUBLIC_API_URL'];
const configuredKey: unknown = process.env['EXPO_PUBLIC_CLERK_PUBLISHABLE_KEY'];
{% if context.auth_oidc %}const configuredIssuer: unknown = process.env['EXPO_PUBLIC_OIDC_ISSUER'];
const configuredClientId: unknown = process.env['EXPO_PUBLIC_OIDC_CLIENT_ID'];
const configuredAudience: unknown = process.env['EXPO_PUBLIC_OIDC_AUDIENCE'];
const configuredResource: unknown = process.env['EXPO_PUBLIC_OIDC_RESOURCE'];
const configuredScopes: unknown = process.env['EXPO_PUBLIC_OIDC_SCOPES'];
{% endif %}{% if not context.auth_oidc %}const configuredLocalIssuer: unknown = process.env['EXPO_PUBLIC_LOCAL_OIDC_ISSUER'];
{% endif %}
const isQaBuild = process.env['BAUKIT_QA_BUILD'] === '1';
const buildVariant = buildVariantFrom(process.env, PRODUCT_NAME);
{% if not context.auth_oidc %}const localSignInBuild = buildVariant === undefined && (process.env.NODE_ENV !== 'production' || isQaBuild);
{% endif %}
function environmentText(name: string): string {
  const value: unknown = process.env[name];
  return typeof value === 'string' ? value : '';
}

const revenueCat = {
  iosApiKey: environmentText('EXPO_PUBLIC_REVENUECAT_IOS_API_KEY'),
  androidApiKey: environmentText('EXPO_PUBLIC_REVENUECAT_ANDROID_API_KEY'),
  offeringId: environmentText('EXPO_PUBLIC_REVENUECAT_OFFERING_ID'),
};

const baseConfig = ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: PRODUCT_NAME,
  slug: PRODUCT_NAME,
  scheme: PRODUCT_NAME,
  version: '0.1.0',
  orientation: 'portrait',
  userInterfaceStyle: 'automatic',
  plugins: [
    ...(config.plugins ?? []),
{% if context.auth_clerk %}    ['@clerk/expo', { appleSignIn: false }],
{% endif %}
    ...(isQaBuild ? ['./plugins/with-qa-local-network.cjs'] : []),
  ],
  extra: {
    ...(typeof configuredApiUrl === 'string' ? { apiBaseUrl: configuredApiUrl } : {}),
    revenueCat,
    clerkPublishableKey: typeof configuredKey === 'string' ? configuredKey : '',
{% if not context.auth_oidc %}    ...(localSignInBuild && typeof configuredLocalIssuer === 'string' && configuredLocalIssuer !== ''
      ? { localOidcIssuer: configuredLocalIssuer }
      : {}),
{% endif %}
{% if context.auth_oidc %}    oidcIssuer:
      typeof configuredIssuer === 'string'
        ? configuredIssuer
        : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
    oidcClientId:
      typeof configuredClientId === 'string' ? configuredClientId : MOBILE_OIDC_CLIENT_ID,
{% if context.pwa and not context.web %}    oidcWebClientId: environmentText('EXPO_PUBLIC_OIDC_WEB_CLIENT_ID') || `${PRODUCT_NAME}-web`,
{% endif %}{% elif context.auth_workos %}    workosClientId: environmentText('EXPO_PUBLIC_WORKOS_CLIENT_ID'),
    workosApiHostname: environmentText('EXPO_PUBLIC_WORKOS_API_HOSTNAME') || 'api.workos.com',
    workosRedirectUri: environmentText('EXPO_PUBLIC_WORKOS_REDIRECT_URI') || `${PRODUCT_NAME}://${OIDC_REDIRECT_PATH}`,
{% endif %}{% if context.auth_oidc %}    oidcAudience: typeof configuredAudience === 'string' ? configuredAudience : undefined,
    oidcResource: typeof configuredResource === 'string' ? configuredResource : undefined,
    oidcScopes: typeof configuredScopes === 'string' ? configuredScopes : undefined,
    oidcOfflineAccess: process.env['EXPO_PUBLIC_OIDC_OFFLINE_ACCESS'] !== 'false',
{% endif %}
  },
  ios: {
    bundleIdentifier: `dev.baukit.${PRODUCT_NAME}`,
    supportsTablet: true,
  },
  android: { package: `dev.baukit.${PRODUCT_NAME.replaceAll('-', '_')}` },
{% if context.pwa and not context.web %}  web: { ...config.web, bundler: 'metro', output: 'single' },
{% endif %}});

{% if context.auth_oidc %}function labSignIn(build: BuildVariant): Record<string, unknown> {
  return { oidcIssuer: build.oidcIssuer, oidcClientId: build.oidcClientId };
}
{% else %}function labSignIn(build: BuildVariant): Record<string, unknown> {
  return { labOidcIssuer: build.oidcIssuer, labOidcClientId: build.oidcClientId };
}
{% endif %}
export default (context: ConfigContext): ExpoConfig =>
  buildVariant === undefined
    ? baseConfig(context)
    : withBuildVariant(baseConfig(context), buildVariant, labSignIn(buildVariant));

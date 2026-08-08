import type { ConfigContext, ExpoConfig } from 'expo/config';

const configuredApiUrl: unknown = process.env['EXPO_PUBLIC_API_URL'];
const configuredIssuer: unknown = process.env['EXPO_PUBLIC_OIDC_ISSUER'];
const configuredClientId: unknown = process.env['EXPO_PUBLIC_OIDC_CLIENT_ID'];

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: '{{ context.app_name }}',
  slug: '{{ context.app_name }}',
  scheme: '{{ context.app_name }}',
  version: '0.1.0',
  orientation: 'portrait',
  userInterfaceStyle: 'automatic',
  extra: {
    apiBaseUrl: typeof configuredApiUrl === 'string' ? configuredApiUrl : 'http://localhost:8080',
    oidcIssuer:
      typeof configuredIssuer === 'string'
        ? configuredIssuer
        : 'http://localhost:8081/realms/{{ context.app_name }}',
    oidcClientId:
      typeof configuredClientId === 'string' ? configuredClientId : '{{ context.app_name }}-mobile',
  },
  ios: { supportsTablet: true },
  android: { package: 'dev.baukit.{{ context.app_crate }}' },
});

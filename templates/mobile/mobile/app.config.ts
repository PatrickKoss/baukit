import type { ConfigContext, ExpoConfig } from 'expo/config';

const configuredApiUrl: unknown = process.env['EXPO_PUBLIC_API_URL'];

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: '{{ context.app_name }}',
  slug: '{{ context.app_name }}',
  scheme: '{{ context.app_name }}',
  version: '0.1.0',
  orientation: 'portrait',
  userInterfaceStyle: 'automatic',
  extra: {
    apiBaseUrl: typeof configuredApiUrl === 'string' ? configuredApiUrl : 'http://localhost:{{ context.api_host_port }}',
  },
  ios: {
    bundleIdentifier: 'dev.baukit.{{ context.app_name }}',
    supportsTablet: true,
  },
  android: {
    package: 'dev.baukit.{{ context.app_crate }}',
  },
});

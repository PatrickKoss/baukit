import type { ConfigContext, ExpoConfig } from 'expo/config';

import { PRODUCT_NAME } from './src/product.ts';

const configuredApiUrl: unknown = process.env['EXPO_PUBLIC_API_URL'];
const isQaBuild = process.env['BAUKIT_QA_BUILD'] === '1';

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: PRODUCT_NAME,
  slug: PRODUCT_NAME,
  scheme: PRODUCT_NAME,
  version: '0.1.0',
  orientation: 'portrait',
  userInterfaceStyle: 'automatic',
  plugins: [
    ...(config.plugins ?? []),
    ...(isQaBuild ? ['./plugins/with-qa-local-network.cjs'] : []),
  ],
  extra: {
    apiBaseUrl:
      typeof configuredApiUrl === 'string'
        ? configuredApiUrl
        : 'http://localhost:{{ context.api_host_port }}',
  },
  ios: {
    bundleIdentifier: `dev.baukit.${PRODUCT_NAME}`,
    supportsTablet: true,
  },
  android: {
    package: `dev.baukit.${PRODUCT_NAME.replaceAll('-', '_')}`,
  },
{% if context.pwa and not context.web %}  web: { ...config.web, bundler: 'metro', output: 'single' },
{% endif %}});

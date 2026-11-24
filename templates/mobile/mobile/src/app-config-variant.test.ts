import type { ConfigContext, ExpoConfig } from 'expo/config';

import { PRODUCT_NAME } from './product';

const variables = [
  'NODE_ENV',
  'APP_VARIANT',
  'BAUKIT_LAB_DOMAIN',
  'BAUKIT_QA_BUILD',
  'BAUKIT_VERSION_CODE',
  'GITHUB_RUN_NUMBER',
  'EXPO_PUBLIC_API_URL',
  'EXPO_PUBLIC_LAB_OIDC_ISSUER',
  'EXPO_PUBLIC_LAB_OIDC_CLIENT_ID',
  'EXPO_PUBLIC_LOCAL_OIDC_ISSUER',
];
const labDomain = 'lab.example.com';
{% if context.auth_enabled %}const labRealm = `https://id.${labDomain}/realms/${PRODUCT_NAME}`;
{% endif %}const androidPackage = `dev.baukit.${PRODUCT_NAME.replaceAll('-', '_')}`;

type AppConfig = (context: ConfigContext) => ExpoConfig;

function configFor(environment: Record<string, string>): ExpoConfig {
  const saved = process.env;
  process.env = {
    ...Object.fromEntries(Object.entries(saved).filter(([name]) => !variables.includes(name))),
    NODE_ENV: 'production',
    ...environment,
  };
  try {
    let appConfig: AppConfig | undefined;
    jest.isolateModules(() => {
      appConfig = jest.requireActual<{ default: AppConfig }>('../app.config').default;
    });
    if (appConfig === undefined) throw new Error('app.config.ts has no default export');
    return appConfig({ projectRoot: process.cwd(), staticConfigPath: null, packageJsonPath: null, config: {} });
  } finally {
    process.env = saved;
  }
}

describe('app config variants', () => {
  it('keeps a plain build without a version code or badge', () => {
    const config = configFor({});
    expect(config.name).toBe(PRODUCT_NAME);
    expect(config.android).toEqual({ package: androidPackage });
    expect(config.plugins).not.toContain('./plugins/with-variant-badge.cjs');
  });

  it('builds dev against the testing tenant and the lab realm', () => {
    const config = configFor({
      APP_VARIANT: 'dev',
      BAUKIT_LAB_DOMAIN: labDomain,
      BAUKIT_VERSION_CODE: '31',
      EXPO_PUBLIC_API_URL: 'http://localhost:8080',
      EXPO_PUBLIC_LOCAL_OIDC_ISSUER: 'http://localhost:8180/realms/local',
    });
    expect(config.name).toBe(`${PRODUCT_NAME} Dev`);
    expect(config.scheme).toBe(`${PRODUCT_NAME}-dev`);
    expect(config.android).toEqual({ package: `${androidPackage}.dev`, versionCode: 31 });
    expect(config.plugins).toContain('./plugins/with-variant-badge.cjs');
    expect(config.extra).toMatchObject({ apiBaseUrl: `https://${PRODUCT_NAME}-testing.${labDomain}` });
{% if context.auth_oidc %}    expect(config.extra).toMatchObject({ oidcIssuer: labRealm, oidcClientId: `${PRODUCT_NAME}-mobile` });
{% elif context.auth_enabled %}    expect(config.extra).toMatchObject({ labOidcIssuer: labRealm, labOidcClientId: `${PRODUCT_NAME}-mobile` });
    expect(config.extra?.['localOidcIssuer']).toBeUndefined();
{% endif %}  });

  it('builds lab under the plain identity with the API URL from the environment', () => {
    const config = configFor({
      APP_VARIANT: 'lab',
      BAUKIT_LAB_DOMAIN: labDomain,
      EXPO_PUBLIC_API_URL: 'https://api.example.test',
    });
    expect(config.name).toBe(PRODUCT_NAME);
    expect(config.scheme).toBe(PRODUCT_NAME);
    expect(config.android).toEqual({ package: androidPackage, versionCode: 1 });
    expect(config.extra).toMatchObject({ apiBaseUrl: 'https://api.example.test' });
  });

  it('refuses a lab build without an API URL', () => {
    expect(() => configFor({ APP_VARIANT: 'lab', BAUKIT_LAB_DOMAIN: labDomain })).toThrow(
      'APP_VARIANT=lab needs EXPO_PUBLIC_API_URL',
    );
  });

  it('refuses a variant build without the lab domain', () => {
    expect(() => configFor({ APP_VARIANT: 'dev' })).toThrow('APP_VARIANT builds need BAUKIT_LAB_DOMAIN');
  });
});

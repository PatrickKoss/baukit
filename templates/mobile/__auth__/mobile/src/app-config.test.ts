import type { ConfigContext, ExpoConfig } from 'expo/config';

const localIssuer = 'http://localhost:{{ context.keycloak_host_port }}/realms/{{ context.app_name }}';
const variables = ['NODE_ENV', 'BAUKIT_QA_BUILD', 'EXPO_PUBLIC_LOCAL_OIDC_ISSUER', 'EXPO_PUBLIC_API_URL'];

type AppConfig = (context: ConfigContext) => ExpoConfig;

function extraFor(environment: Record<string, string>): Record<string, unknown> {
  const saved = process.env;
  process.env = {
    ...Object.fromEntries(Object.entries(saved).filter(([name]) => !variables.includes(name))),
    ...environment,
  } as NodeJS.ProcessEnv;
  try {
    let appConfig: AppConfig | undefined;
    jest.isolateModules(() => {
      appConfig = jest.requireActual<{ default: AppConfig }>('../app.config').default;
    });
    if (appConfig === undefined) throw new Error('app.config.ts has no default export');
    return appConfig({ config: {} } as ConfigContext).extra ?? {};
  } finally {
    process.env = saved;
  }
}

const cases: readonly [string, Record<string, string>, boolean][] = [
  ['development', { NODE_ENV: 'development', EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, true],
  ['production', { NODE_ENV: 'production', EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, false],
  [
    'production QA',
    { NODE_ENV: 'production', BAUKIT_QA_BUILD: '1', EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer },
    true,
  ],
  [
    'production with a QA flag other than 1',
    { NODE_ENV: 'production', BAUKIT_QA_BUILD: 'true', EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer },
    false,
  ],
];

describe('app config', () => {
  it.each(cases)('a %s build carries the local realm only when local sign-in is on', (_name, environment, local) => {
    const extra = extraFor(environment);
    expect(extra['localOidcIssuer']).toBe(local ? localIssuer : undefined);
    expect(extra['apiBaseUrl']).toBeUndefined();
  });

  it('passes a configured API URL through', () => {
    const extra = extraFor({ NODE_ENV: 'production', EXPO_PUBLIC_API_URL: 'https://api.example.test' });
    expect(extra['apiBaseUrl']).toBe('https://api.example.test');
  });
});

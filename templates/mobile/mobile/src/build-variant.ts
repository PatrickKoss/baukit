import type { ExpoConfig } from 'expo/config';

import { MOBILE_OIDC_CLIENT_ID } from './product.ts';

export const DEFAULT_VERSION_CODE = 1;
const MAX_VERSION_CODE = 2_100_000_000;
const VARIANTS = ['dev', 'lab'] as const;
const LAB_DOMAIN_VARIABLE = 'BAUKIT_LAB_DOMAIN';
const HOSTNAME = /^(?=.{1,253}$)[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$/u;
export const DEV_SCHEME_SUFFIX = '-dev';
const DEV_BADGE_PLUGIN = './plugins/with-variant-badge.cjs';

export type AppVariant = (typeof VARIANTS)[number];

type BuildEnvironment = Readonly<Record<string, string | undefined>>;

export interface BuildVariant {
  readonly variant: AppVariant;
  readonly labDomain: string;
  readonly apiUrl: string;
  readonly versionCode: number;
  readonly oidcIssuer: string;
  readonly oidcClientId: string;
}

function text(environment: BuildEnvironment, name: string): string {
  return environment[name]?.trim() ?? '';
}

function isVariant(value: string): value is AppVariant {
  return (VARIANTS as readonly string[]).includes(value);
}

function httpsUrl(name: string, value: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${name} must be an https URL, got "${value}"`);
  }
  if (url.protocol !== 'https:') throw new Error(`${name} must be an https URL, got "${value}"`);
  return value;
}

function labDomainFrom(environment: BuildEnvironment): string {
  const domain = text(environment, LAB_DOMAIN_VARIABLE);
  if (domain === '') {
    throw new Error(`APP_VARIANT builds need ${LAB_DOMAIN_VARIABLE}, the lab's DNS name such as lab.example.com`);
  }
  if (!HOSTNAME.test(domain)) {
    throw new Error(`${LAB_DOMAIN_VARIABLE} must be a lowercase DNS name without scheme or path, got "${domain}"`);
  }
  return domain;
}

function apiUrlFor(variant: AppVariant, environment: BuildEnvironment, productName: string, labDomain: string): string {
  if (variant === 'dev') return `https://${productName}-testing.${labDomain}`;
  const configured = text(environment, 'EXPO_PUBLIC_API_URL');
  if (configured === '') {
    throw new Error('APP_VARIANT=lab needs EXPO_PUBLIC_API_URL, the https origin of the product on the production cluster');
  }
  return httpsUrl('EXPO_PUBLIC_API_URL', configured);
}

function versionCodeFrom(environment: BuildEnvironment): number {
  const configured = text(environment, 'BAUKIT_VERSION_CODE') || text(environment, 'GITHUB_RUN_NUMBER');
  if (configured === '') return DEFAULT_VERSION_CODE;
  const versionCode = Number(configured);
  if (!/^\d+$/u.test(configured) || versionCode < 1 || versionCode > MAX_VERSION_CODE) {
    throw new Error(`the version code must be a whole number from 1 to ${String(MAX_VERSION_CODE)}, got "${configured}"`);
  }
  return versionCode;
}

/** Reads `APP_VARIANT` and the settings that go with it; `undefined` means a plain build. */
export function buildVariantFrom(environment: BuildEnvironment, productName: string): BuildVariant | undefined {
  const variant = text(environment, 'APP_VARIANT');
  if (variant === '') return undefined;
  if (!isVariant(variant)) throw new Error(`APP_VARIANT must be one of ${VARIANTS.join(', ')}, got "${variant}"`);
  if (environment['BAUKIT_QA_BUILD'] === '1') {
    throw new Error('APP_VARIANT and BAUKIT_QA_BUILD=1 cannot be combined: QA builds sign in against the local realm');
  }
  const labDomain = labDomainFrom(environment);
  return {
    variant,
    labDomain,
    apiUrl: apiUrlFor(variant, environment, productName, labDomain),
    versionCode: versionCodeFrom(environment),
    oidcIssuer: httpsUrl(
      'EXPO_PUBLIC_LAB_OIDC_ISSUER',
      text(environment, 'EXPO_PUBLIC_LAB_OIDC_ISSUER') || `https://id.${labDomain}/realms/${productName}`,
    ),
    oidcClientId: text(environment, 'EXPO_PUBLIC_LAB_OIDC_CLIENT_ID') || MOBILE_OIDC_CLIENT_ID,
  };
}

/** Applies a variant's identity, deep-link scheme, version code and API URL; `signIn` lands in `extra`. */
export function withBuildVariant(
  config: ExpoConfig,
  build: BuildVariant,
  signIn: Readonly<Record<string, unknown>>,
): ExpoConfig {
  const dev = build.variant === 'dev';
  const suffix = dev ? '.dev' : '';
  const android = config.android ?? {};
  const ios = config.ios ?? {};
  return {
    ...config,
    name: dev ? `${config.name} Dev` : config.name,
    ...(dev && typeof config.scheme === 'string' ? { scheme: `${config.scheme}${DEV_SCHEME_SUFFIX}` } : {}),
    plugins: [...(config.plugins ?? []), ...(dev ? [DEV_BADGE_PLUGIN] : [])],
    extra: { ...config.extra, apiBaseUrl: build.apiUrl, ...signIn },
    ios: {
      ...ios,
      ...(ios.bundleIdentifier === undefined ? {} : { bundleIdentifier: `${ios.bundleIdentifier}${suffix}` }),
      buildNumber: String(build.versionCode),
    },
    android: {
      ...android,
      ...(android.package === undefined ? {} : { package: `${android.package}${suffix}` }),
      versionCode: build.versionCode,
    },
  };
}

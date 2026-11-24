import type { ExpoConfig } from 'expo/config';

import { buildVariantFrom, DEFAULT_VERSION_CODE, LAB_DOMAIN, withBuildVariant } from './build-variant';

const product = 'demo';
const labRealm = `https://id.${LAB_DOMAIN}/realms/${product}`;
const plain: ExpoConfig = {
  name: product,
  slug: product,
  plugins: ['expo-router'],
  extra: { revenueCat: {} },
  ios: { bundleIdentifier: `dev.baukit.${product}`, supportsTablet: true },
  android: { package: `dev.baukit.${product}` },
};

describe('build variant', () => {
  it('is absent when APP_VARIANT is unset or empty', () => {
    expect(buildVariantFrom({}, product)).toBeUndefined();
    expect(buildVariantFrom({ APP_VARIANT: ' ' }, product)).toBeUndefined();
  });

  it('rejects an unknown variant', () => {
    expect(() => buildVariantFrom({ APP_VARIANT: 'prod' }, product)).toThrow('APP_VARIANT must be one of dev, lab');
  });

  it('rejects a variant on a QA build', () => {
    expect(() => buildVariantFrom({ APP_VARIANT: 'dev', BAUKIT_QA_BUILD: '1' }, product)).toThrow('BAUKIT_QA_BUILD');
  });

  it('points dev at the testing tenant and the lab realm, ignoring the local API URL', () => {
    expect(buildVariantFrom({ APP_VARIANT: 'dev', EXPO_PUBLIC_API_URL: 'http://localhost:8080' }, product)).toEqual({
      variant: 'dev',
      apiUrl: `https://${product}-testing.${LAB_DOMAIN}`,
      versionCode: DEFAULT_VERSION_CODE,
      oidcIssuer: labRealm,
      oidcClientId: `${product}-mobile`,
    });
  });

  it('needs an https API URL for lab', () => {
    expect(() => buildVariantFrom({ APP_VARIANT: 'lab' }, product)).toThrow('APP_VARIANT=lab needs EXPO_PUBLIC_API_URL');
    expect(() => buildVariantFrom({ APP_VARIANT: 'lab', EXPO_PUBLIC_API_URL: 'http://localhost:8080' }, product)).toThrow(
      'EXPO_PUBLIC_API_URL must be an https URL',
    );
    expect(() => buildVariantFrom({ APP_VARIANT: 'lab', EXPO_PUBLIC_API_URL: 'not a url' }, product)).toThrow(
      'EXPO_PUBLIC_API_URL must be an https URL',
    );
    expect(buildVariantFrom({ APP_VARIANT: 'lab', EXPO_PUBLIC_API_URL: 'https://demo.example.test' }, product)).toMatchObject({
      variant: 'lab',
      apiUrl: 'https://demo.example.test',
    });
  });

  it('takes the issuer and client from the build environment', () => {
    const variant = buildVariantFrom(
      {
        APP_VARIANT: 'dev',
        EXPO_PUBLIC_LAB_OIDC_ISSUER: 'https://id.example.test/realms/other',
        EXPO_PUBLIC_LAB_OIDC_CLIENT_ID: 'other-mobile',
      },
      product,
    );
    expect(variant).toMatchObject({ oidcIssuer: 'https://id.example.test/realms/other', oidcClientId: 'other-mobile' });
    expect(() =>
      buildVariantFrom({ APP_VARIANT: 'dev', EXPO_PUBLIC_LAB_OIDC_ISSUER: 'http://localhost:8180/realms/demo' }, product),
    ).toThrow('EXPO_PUBLIC_LAB_OIDC_ISSUER must be an https URL');
  });

  it.each([
    [{ BAUKIT_VERSION_CODE: '42' }, 42],
    [{ GITHUB_RUN_NUMBER: '7' }, 7],
    [{ BAUKIT_VERSION_CODE: '42', GITHUB_RUN_NUMBER: '7' }, 42],
  ])('reads the version code from %o', (environment, versionCode) => {
    expect(buildVariantFrom({ APP_VARIANT: 'dev', ...environment }, product)?.versionCode).toBe(versionCode);
  });

  it.each(['0', '-1', '1.5', 'abc', '2100000001'])('rejects the version code %s', (versionCode) => {
    expect(() => buildVariantFrom({ APP_VARIANT: 'dev', BAUKIT_VERSION_CODE: versionCode }, product)).toThrow(
      'the version code must be a whole number',
    );
  });

  it('gives dev its own identity, name, badge and version code', () => {
    const variant = buildVariantFrom({ APP_VARIANT: 'dev', BAUKIT_VERSION_CODE: '12' }, product);
    if (variant === undefined) throw new Error('expected a variant');
    expect(withBuildVariant(plain, variant, { labOidcIssuer: labRealm })).toEqual({
      ...plain,
      name: `${product} Dev`,
      plugins: ['expo-router', './plugins/with-variant-badge.cjs'],
      extra: { revenueCat: {}, apiBaseUrl: `https://${product}-testing.${LAB_DOMAIN}`, labOidcIssuer: labRealm },
      ios: { bundleIdentifier: `dev.baukit.${product}.dev`, supportsTablet: true, buildNumber: '12' },
      android: { package: `dev.baukit.${product}.dev`, versionCode: 12 },
    });
  });

  it('keeps the plain identity for lab and tolerates a config without native sections', () => {
    const variant = buildVariantFrom({ APP_VARIANT: 'lab', EXPO_PUBLIC_API_URL: 'https://demo.example.test' }, product);
    if (variant === undefined) throw new Error('expected a variant');
    const lab = withBuildVariant(plain, variant, {});
    expect(lab.name).toBe(product);
    expect(lab.plugins).toEqual(['expo-router']);
    expect(lab.android).toEqual({ package: `dev.baukit.${product}`, versionCode: DEFAULT_VERSION_CODE });
    expect(lab.ios?.bundleIdentifier).toBe(`dev.baukit.${product}`);
    const bare = withBuildVariant({ name: product, slug: product }, variant, {});
    expect(bare.android).toEqual({ versionCode: DEFAULT_VERSION_CODE });
    expect(bare.ios).toEqual({ buildNumber: String(DEFAULT_VERSION_CODE) });
    expect(bare.plugins).toEqual([]);
  });
});

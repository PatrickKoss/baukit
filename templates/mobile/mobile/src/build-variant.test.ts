import type { ExpoConfig } from 'expo/config';

import { buildVariantFrom, DEFAULT_VERSION_CODE, withBuildVariant } from './build-variant';

const product = 'demo';
const domain = 'lab.example.com';
const labRealm = `https://id.${domain}/realms/${product}`;
const dev = { APP_VARIANT: 'dev', BAUKIT_LAB_DOMAIN: domain };
const lab = { APP_VARIANT: 'lab', BAUKIT_LAB_DOMAIN: domain, EXPO_PUBLIC_API_URL: 'https://demo.example.test' };
const plain: ExpoConfig = {
  name: product,
  slug: product,
  scheme: product,
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
    expect(() => buildVariantFrom({ ...dev, APP_VARIANT: 'prod' }, product)).toThrow('APP_VARIANT must be one of dev, lab');
  });

  it('rejects a variant on a QA build', () => {
    expect(() => buildVariantFrom({ ...dev, BAUKIT_QA_BUILD: '1' }, product)).toThrow('BAUKIT_QA_BUILD');
  });

  it('needs the lab domain', () => {
    expect(() => buildVariantFrom({ APP_VARIANT: 'dev' }, product)).toThrow('APP_VARIANT builds need BAUKIT_LAB_DOMAIN');
    expect(() => buildVariantFrom({ ...lab, BAUKIT_LAB_DOMAIN: '' }, product)).toThrow('need BAUKIT_LAB_DOMAIN');
  });

  it.each(['https://lab.example.com', 'lab.example.com/realms', 'Lab.Example.com', 'localhost', 'lab..example.com', '-lab.example.com'])(
    'rejects the lab domain %s',
    (value) => {
      expect(() => buildVariantFrom({ ...dev, BAUKIT_LAB_DOMAIN: value }, product)).toThrow(
        'BAUKIT_LAB_DOMAIN must be a lowercase DNS name without scheme or path',
      );
    },
  );

  it('points dev at the testing tenant and the lab realm, ignoring the local API URL', () => {
    expect(buildVariantFrom({ ...dev, EXPO_PUBLIC_API_URL: 'http://localhost:8080' }, product)).toEqual({
      variant: 'dev',
      labDomain: domain,
      apiUrl: `https://${product}-testing.${domain}`,
      versionCode: DEFAULT_VERSION_CODE,
      oidcIssuer: labRealm,
      oidcClientId: `${product}-mobile`,
    });
  });

  it('needs an https API URL for lab', () => {
    expect(() => buildVariantFrom({ ...lab, EXPO_PUBLIC_API_URL: '' }, product)).toThrow(
      'APP_VARIANT=lab needs EXPO_PUBLIC_API_URL',
    );
    expect(() => buildVariantFrom({ ...lab, EXPO_PUBLIC_API_URL: 'http://localhost:8080' }, product)).toThrow(
      'EXPO_PUBLIC_API_URL must be an https URL',
    );
    expect(() => buildVariantFrom({ ...lab, EXPO_PUBLIC_API_URL: 'not a url' }, product)).toThrow(
      'EXPO_PUBLIC_API_URL must be an https URL',
    );
    expect(buildVariantFrom(lab, product)).toMatchObject({ variant: 'lab', apiUrl: 'https://demo.example.test' });
  });

  it('takes the issuer and client from the build environment', () => {
    const variant = buildVariantFrom(
      {
        ...dev,
        EXPO_PUBLIC_LAB_OIDC_ISSUER: 'https://id.example.test/realms/other',
        EXPO_PUBLIC_LAB_OIDC_CLIENT_ID: 'other-mobile',
      },
      product,
    );
    expect(variant).toMatchObject({ oidcIssuer: 'https://id.example.test/realms/other', oidcClientId: 'other-mobile' });
    expect(() =>
      buildVariantFrom({ ...dev, EXPO_PUBLIC_LAB_OIDC_ISSUER: 'http://localhost:8180/realms/demo' }, product),
    ).toThrow('EXPO_PUBLIC_LAB_OIDC_ISSUER must be an https URL');
  });

  it.each([
    [{ BAUKIT_VERSION_CODE: '42' }, 42],
    [{ GITHUB_RUN_NUMBER: '7' }, 7],
    [{ BAUKIT_VERSION_CODE: '42', GITHUB_RUN_NUMBER: '7' }, 42],
  ])('reads the version code from %o', (environment, versionCode) => {
    expect(buildVariantFrom({ ...dev, ...environment }, product)?.versionCode).toBe(versionCode);
  });

  it.each(['0', '-1', '1.5', 'abc', '2100000001'])('rejects the version code %s', (versionCode) => {
    expect(() => buildVariantFrom({ ...dev, BAUKIT_VERSION_CODE: versionCode }, product)).toThrow(
      'the version code must be a whole number',
    );
  });

  it('gives dev its own identity, scheme, name, badge and version code', () => {
    const variant = buildVariantFrom({ ...dev, BAUKIT_VERSION_CODE: '12' }, product);
    if (variant === undefined) throw new Error('expected a variant');
    expect(withBuildVariant(plain, variant, { labOidcIssuer: labRealm })).toEqual({
      ...plain,
      name: `${product} Dev`,
      scheme: `${product}-dev`,
      plugins: ['expo-router', './plugins/with-variant-badge.cjs'],
      extra: { revenueCat: {}, apiBaseUrl: `https://${product}-testing.${domain}`, labOidcIssuer: labRealm },
      ios: { bundleIdentifier: `dev.baukit.${product}.dev`, supportsTablet: true, buildNumber: '12' },
      android: { package: `dev.baukit.${product}.dev`, versionCode: 12 },
    });
  });

  it('keeps the plain identity for lab and tolerates a config without native sections', () => {
    const variant = buildVariantFrom(lab, product);
    if (variant === undefined) throw new Error('expected a variant');
    const built = withBuildVariant(plain, variant, {});
    expect(built.name).toBe(product);
    expect(built.scheme).toBe(product);
    expect(built.plugins).toEqual(['expo-router']);
    expect(built.android).toEqual({ package: `dev.baukit.${product}`, versionCode: DEFAULT_VERSION_CODE });
    expect(built.ios?.bundleIdentifier).toBe(`dev.baukit.${product}`);
    const bare = withBuildVariant({ name: product, slug: product }, variant, {});
    expect(bare.android).toEqual({ versionCode: DEFAULT_VERSION_CODE });
    expect(bare.ios).toEqual({ buildNumber: String(DEFAULT_VERSION_CODE) });
    expect(bare.plugins).toEqual([]);
    expect(bare.scheme).toBeUndefined();
  });
});

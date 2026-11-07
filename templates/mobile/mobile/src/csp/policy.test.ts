import { policyOrigins } from './origins';
import {
  contentSecurityPolicy,
  developmentContentSecurityPolicy,
  hashSource,
  originOf,
  unhashedInlineScripts,
} from './policy';

function directive(policy: string, name: string): string[] | undefined {
  return policy
    .split(';')
    .map((part) => part.trim().split(' '))
    .find(([directiveName]) => directiveName === name)
    ?.slice(1);
}

const hashOf = (script: string): string => (script === 'boot()' ? 'Ym9vdA==' : 'b3RoZXI=');

describe('web export content security policy', () => {
  it('resolves origins from the public Expo env', () => {
    const origins = policyOrigins({
      EXPO_PUBLIC_API_URL: 'https://api.example.test/v1',
      EXPO_PUBLIC_POSTHOG_HOST: 'eu.posthog.example.test',
      EXPO_PUBLIC_MEDIA_ORIGIN: ' ',
    });

    expect(origins.api).toBe('https://api.example.test');
    expect(origins.analytics).toBe('https://eu.posthog.example.test');
    expect(origins.media).toBeUndefined();
  });

  it('allows runtime-injected styles but no inline scripts', () => {
    const policy = contentSecurityPolicy({
      styles: 'runtime-injected',
      origins: { api: 'https://api.example.test', media: 'https://media.example.test' },
      scriptHashes: [hashSource('Ym9vdA==')],
    });

    expect(directive(policy, 'style-src')).toEqual(["'self'", "'unsafe-inline'"]);
    expect(directive(policy, 'script-src')).toEqual(["'self'", "'sha256-Ym9vdA=='"]);
    expect(directive(policy, 'img-src')).toContain('https://media.example.test');
    expect(directive(policy, 'frame-src')).toEqual(["'none'"]);
    expect(policy).toContain('upgrade-insecure-requests');
  });

  it('rejects anything but hash sources for scripts', () => {
    expect(() => hashSource('not base64!')).toThrow(/SHA-256/u);
    expect(() =>
      contentSecurityPolicy({ styles: 'files', origins: {}, scriptHashes: ["'unsafe-inline'"] }),
    ).toThrow(/hash source/u);
  });

  it('reports inline scripts the policy does not hash', () => {
    const policy = contentSecurityPolicy({
      styles: 'files',
      origins: { issuer: 'http://localhost:8081' },
      scriptHashes: [hashSource('Ym9vdA==')],
    });

    expect(policy).not.toContain('upgrade-insecure-requests');
    expect(
      unhashedInlineScripts('<script>boot()</script><script>other()</script>', policy, hashOf),
    ).toEqual(["'sha256-b3RoZXI='"]);
  });

  it('keeps the relaxed variant for development only', () => {
    const policy = developmentContentSecurityPolicy(
      { styles: 'files', origins: {}, scriptHashes: [] },
      ['ws://localhost:8081'],
    );

    expect(directive(policy, 'connect-src')).toEqual(["'self'", 'ws://localhost:8081']);
    expect(originOf('https://example.test/path')).toBe('https://example.test');
  });
});

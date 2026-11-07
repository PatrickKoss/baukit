import { createHash } from 'node:crypto';

import { describe, expect, it } from 'vitest';

import {
  clerkFrontendApiOrigin,
  contentSecurityPolicy,
  developmentContentSecurityPolicy,
  hashSource,
  inlineScripts,
  originOf,
  SECURITY_HEADERS,
  unhashedInlineScripts,
} from './index.js';

const origins = {
  api: 'https://api.example.test',
  issuer: 'https://login.example.test',
  analytics: 'https://eu.posthog.example.test',
};

function sha256Base64(text: string): string {
  return createHash('sha256').update(text, 'utf8').digest('base64');
}

function directive(policy: string, name: string): string[] | undefined {
  return policy
    .split(';')
    .map((part) => part.trim().split(' '))
    .find(([directiveName]) => directiveName === name)
    ?.slice(1);
}

describe('content security policy', () => {
  it('locks scripts and styles to the app origin for file-based CSS', () => {
    const policy = contentSecurityPolicy({ styles: 'files', origins, scriptHashes: [] });

    expect(directive(policy, 'script-src')).toEqual(["'self'"]);
    expect(directive(policy, 'style-src')).toEqual(["'self'"]);
    expect(directive(policy, 'connect-src')).toEqual([
      "'self'",
      'https://api.example.test',
      'https://login.example.test',
      'https://eu.posthog.example.test',
    ]);
    expect(directive(policy, 'frame-src')).toEqual(['https://login.example.test']);
    expect(directive(policy, 'form-action')).toEqual(["'self'", 'https://login.example.test']);
    expect(directive(policy, 'object-src')).toEqual(["'none'"]);
    expect(directive(policy, 'base-uri')).toEqual(["'none'"]);
    expect(directive(policy, 'frame-ancestors')).toEqual(["'none'"]);
    expect(policy).toContain('upgrade-insecure-requests');
    expect(policy).not.toContain('unsafe-inline');
    expect(policy).not.toContain('unsafe-eval');
  });

  it('allows inline styles but never inline scripts for runtime-injected CSS', () => {
    const policy = contentSecurityPolicy({
      styles: 'runtime-injected',
      origins: { api: 'https://api.example.test', media: 'https://media.example.test' },
      scriptHashes: [hashSource('Ym9vdA==')],
    });

    expect(directive(policy, 'style-src')).toEqual(["'self'", "'unsafe-inline'"]);
    expect(directive(policy, 'script-src')).toEqual(["'self'", "'sha256-Ym9vdA=='"]);
    expect(directive(policy, 'img-src')).toContain('https://media.example.test');
    expect(directive(policy, 'frame-src')).toEqual(["'none'"]);
  });

  it('rejects anything but hash sources for scripts', () => {
    expect(() => hashSource('not base64!')).toThrow(/SHA-256/u);
    expect(() =>
      contentSecurityPolicy({ styles: 'files', origins, scriptHashes: ["'unsafe-inline'"] }),
    ).toThrow(/not a CSP hash source/u);
  });

  it('deduplicates origins configured twice', () => {
    const policy = contentSecurityPolicy({
      styles: 'files',
      origins: { api: 'https://same.example.test', issuer: 'https://same.example.test' },
      scriptHashes: [],
    });

    expect(directive(policy, 'connect-src')).toEqual(["'self'", 'https://same.example.test']);
  });

  it('omits upgrade-insecure-requests only for plain http origins', () => {
    const local = contentSecurityPolicy({
      styles: 'files',
      origins: { vendorImages: ['http://localhost:9000'] },
      scriptHashes: [],
    });

    expect(local).not.toContain('upgrade-insecure-requests');
    expect(directive(local, 'frame-src')).toEqual(["'none'"]);
  });

  it('relaxes only the dev variant for styles and the HMR socket', () => {
    const policy = developmentContentSecurityPolicy(
      { styles: 'files', origins, scriptHashes: [] },
      ['ws://localhost:5173'],
    );

    expect(directive(policy, 'style-src')).toEqual(["'self'", "'unsafe-inline'"]);
    expect(directive(policy, 'connect-src')).toContain('ws://localhost:5173');
    expect(directive(policy, 'script-src')).toEqual(["'self'"]);
    expect(policy).not.toContain('upgrade-insecure-requests');
  });
});

describe('inline script hashes', () => {
  const html =
    '<script type="module" src="/assets/index.js"></script><script>boot()</script><script type="importmap">{}</script>';

  it('finds inline scripts and skips external ones', () => {
    expect(inlineScripts(html)).toEqual(['boot()', '{}']);
  });

  it('reports every inline script the policy does not hash', () => {
    const policy = contentSecurityPolicy({
      styles: 'files',
      origins,
      scriptHashes: [hashSource(sha256Base64('boot()'))],
    });

    expect(unhashedInlineScripts(html, policy, sha256Base64)).toEqual([
      `'sha256-${sha256Base64('{}')}'`,
    ]);
  });
});

describe('origins', () => {
  it('reduces URLs and bare hosts to origins', () => {
    expect(originOf('https://api.example.test/v1/')).toBe('https://api.example.test');
    expect(originOf('eu.posthog.example.test')).toBe('https://eu.posthog.example.test');
    expect(originOf('  ')).toBeUndefined();
    expect(originOf(undefined)).toBeUndefined();
  });

  it('decodes the Clerk frontend API host from a publishable key', () => {
    const key = `pk_test_${btoa('clerk.example.test$')}`;

    expect(clerkFrontendApiOrigin(key)).toBe('https://clerk.example.test');
    expect(clerkFrontendApiOrigin('')).toBeUndefined();
    expect(clerkFrontendApiOrigin(undefined)).toBeUndefined();
  });
});

describe('security headers', () => {
  it('isolates the opener and resources without requiring CORP from subresources', () => {
    expect(SECURITY_HEADERS).toEqual({
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Resource-Policy': 'same-origin',
      'X-Content-Type-Options': 'nosniff',
    });
    expect(Object.keys(SECURITY_HEADERS)).not.toContain('Cross-Origin-Embedder-Policy');
  });
});

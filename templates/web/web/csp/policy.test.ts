import { describe, expect, it } from 'vitest';

import {
  clerkFrontendApiOrigin,
  contentSecurityPolicy,
  developmentContentSecurityPolicy,
  inlineScripts,
  originOf,
  unhashedInlineScripts,
} from './policy.ts';
import { scriptHashSources, sha256Base64 } from './vite-plugin.ts';

const origins = {
  api: 'https://api.example.test',
  issuer: 'https://login.example.test',
  analytics: 'https://eu.posthog.example.test',
};

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
    const policy = contentSecurityPolicy({ styles: 'runtime-injected', origins, scriptHashes: [] });

    expect(directive(policy, 'style-src')).toEqual(["'self'", "'unsafe-inline'"]);
    expect(directive(policy, 'script-src')).toEqual(["'self'"]);
  });

  it('adds hash sources for inline scripts and rejects anything else', () => {
    const hash = scriptHashSources('<script>window.boot = 1;</script>');
    const policy = contentSecurityPolicy({ styles: 'files', origins, scriptHashes: hash });

    expect(directive(policy, 'script-src')).toEqual(["'self'", ...hash]);
    expect(() =>
      contentSecurityPolicy({ styles: 'files', origins, scriptHashes: ["'unsafe-inline'"] }),
    ).toThrow(/not a CSP hash source/u);
  });

  it('omits upgrade-insecure-requests only for plain http origins', () => {
    const local = contentSecurityPolicy({
      styles: 'files',
      origins: { api: 'http://localhost:8080' },
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
  });
});

describe('inline script hashes', () => {
  const html =
    '<script type="module" src="/assets/index.js"></script><script>boot()</script><script type="importmap">{}</script>';

  it('finds inline scripts and skips external ones', () => {
    expect(inlineScripts(html)).toEqual(['boot()', '{}']);
  });

  it('reports every inline script the policy does not hash', () => {
    const [bootHash] = scriptHashSources('<script>boot()</script>');
    const policy = contentSecurityPolicy({
      styles: 'files',
      origins,
      scriptHashes: bootHash === undefined ? [] : [bootHash],
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

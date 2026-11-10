import { describe, expect, it } from 'vitest';

import { assertProductionOrigins, contentSecurityPolicy } from './index.js';

function directive(policy: string, name: string): string[] | undefined {
  return policy
    .split(';')
    .map((part) => part.trim().split(' '))
    .find(([directiveName]) => directiveName === name)
    ?.slice(1);
}

const base = { styles: 'files', scriptHashes: [] } as const;

describe('vendor script and frame sources', () => {
  it('adds vendor script origins to script-src before the hashes', () => {
    const policy = contentSecurityPolicy({
      ...base,
      scriptHashes: ["'sha256-Ym9vdA=='"],
      origins: { vendorScripts: ['https://clerk.example.test'] },
    });

    expect(directive(policy, 'script-src')).toEqual([
      "'self'",
      'https://clerk.example.test',
      "'sha256-Ym9vdA=='",
    ]);
  });

  it('adds vendor frames and blob: to frame-src next to the issuer', () => {
    const policy = contentSecurityPolicy({
      ...base,
      blobFrames: true,
      origins: { issuer: 'https://login.example.test', vendorFrames: ['https://pay.example.test'] },
    });

    expect(directive(policy, 'frame-src')).toEqual([
      'https://login.example.test',
      'https://pay.example.test',
      'blob:',
    ]);
  });

  it('keeps frame-src at none without an issuer, vendor frame or blob frames', () => {
    expect(directive(contentSecurityPolicy({ ...base, origins: {} }), 'frame-src')).toEqual([
      "'none'",
    ]);
    expect(
      directive(contentSecurityPolicy({ ...base, blobFrames: true, origins: {} }), 'frame-src'),
    ).toEqual(['blob:']);
  });

  it('treats a plain http vendor script origin as a local build', () => {
    const policy = contentSecurityPolicy({
      ...base,
      origins: { vendorScripts: ['http://localhost:9000'] },
    });

    expect(policy).not.toContain('upgrade-insecure-requests');
  });
});

describe('production origin guard', () => {
  it('accepts public HTTPS origins', () => {
    expect(() => {
      assertProductionOrigins({
        api: 'https://api.example.test',
        vendorScripts: ['https://clerk.example.test'],
      });
    }).not.toThrow();
  });

  it.each([
    'http://api.example.test',
    'https://user:secret@api.example.test',
    'https://localhost:8080',
    'https://app.localhost',
    'https://127.0.0.1',
    'https://127.1.2.3:9',
    'https://0.0.0.0',
    'https://[::1]',
    'https://[::ffff:7f00:1]',
    'https://localhost.',
  ])('rejects %s', (origin) => {
    expect(() => {
      assertProductionOrigins({ api: origin });
    }).toThrow();
    expect(() => {
      assertProductionOrigins({ vendorFrames: [origin] });
    }).toThrow();
  });

  it('skips unset origins', () => {
    expect(() => {
      assertProductionOrigins({ api: undefined, issuer: undefined });
    }).not.toThrow();
  });
});

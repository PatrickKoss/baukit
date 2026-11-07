import { describe, expect, it } from 'vitest';

import { contentSecurityPolicy, developmentContentSecurityPolicy } from './index.js';

const everyOrigin = {
  api: 'https://api.example.test',
  issuer: 'https://login.example.test',
  analytics: 'https://eu.posthog.example.test',
  media: 'https://media.example.test',
  suiteHub: 'https://hub.example.test',
  vendorConnect: ['https://clerk.example.test'],
  vendorImages: ['https://img.clerk.com'],
};
const bootHash = ["'sha256-Ym9vdA=='"];

// Output of the template copies of policy.ts before they moved into this package.
describe('policy vectors', () => {
  it('keeps the production policy with every origin byte-equal', () => {
    expect(
      contentSecurityPolicy({ styles: 'files', origins: everyOrigin, scriptHashes: bootHash }),
    ).toBe(
      "default-src 'self'; script-src 'self' 'sha256-Ym9vdA=='; style-src 'self'; img-src 'self' data: blob: https://media.example.test https://img.clerk.com; media-src 'self' blob: https://media.example.test; connect-src 'self' https://api.example.test https://login.example.test https://eu.posthog.example.test https://hub.example.test https://clerk.example.test; frame-src https://login.example.test; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self' https://login.example.test; frame-ancestors 'none'; upgrade-insecure-requests",
    );
  });

  it('keeps the local http policy with runtime-injected styles byte-equal', () => {
    expect(
      contentSecurityPolicy({
        styles: 'runtime-injected',
        origins: { api: 'http://localhost:8080', issuer: 'http://localhost:8081' },
        scriptHashes: [],
      }),
    ).toBe(
      "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self' http://localhost:8080 http://localhost:8081; frame-src http://localhost:8081; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self' http://localhost:8081; frame-ancestors 'none'",
    );
  });

  it('keeps the policy without origins byte-equal', () => {
    expect(contentSecurityPolicy({ styles: 'files', origins: {}, scriptHashes: [] })).toBe(
      "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; frame-src 'none'; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; upgrade-insecure-requests",
    );
  });

  it('keeps the development policy byte-equal', () => {
    expect(
      developmentContentSecurityPolicy(
        { styles: 'files', origins: everyOrigin, scriptHashes: bootHash },
        ['ws://localhost:5173', 'ws://127.0.0.1:5173'],
      ),
    ).toBe(
      "default-src 'self'; script-src 'self' 'sha256-Ym9vdA=='; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https://media.example.test https://img.clerk.com; media-src 'self' blob: https://media.example.test; connect-src 'self' https://api.example.test https://login.example.test https://eu.posthog.example.test https://hub.example.test https://clerk.example.test ws://localhost:5173 ws://127.0.0.1:5173; frame-src https://login.example.test; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self' https://login.example.test; frame-ancestors 'none'",
    );
  });
});

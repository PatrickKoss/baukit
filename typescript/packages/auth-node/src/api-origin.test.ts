import { describe, expect, it } from 'vitest';

import { ApiOriginError, parseApiOrigin, type ApiOriginErrorReason } from './index.js';

function reasonFor(value: string, allowLoopbackHttp = false): ApiOriginErrorReason | undefined {
  try {
    parseApiOrigin(value, { allowLoopbackHttp });
    return undefined;
  } catch (cause) {
    if (cause instanceof ApiOriginError) return cause.reason;
    throw cause;
  }
}

describe('parseApiOrigin', () => {
  it('returns the origin of an https URL', () => {
    expect(parseApiOrigin('https://api.example.com')).toBe('https://api.example.com');
    expect(parseApiOrigin('  https://API.example.com:8443/ \n')).toBe(
      'https://api.example.com:8443',
    );
  });

  it('allows plain http only on loopback hosts when asked', () => {
    for (const value of [
      'http://localhost:8080',
      'http://127.0.0.1:8080',
      'http://127.10.0.2',
      'http://[::1]:8080',
    ]) {
      expect(parseApiOrigin(value, { allowLoopbackHttp: true })).toBe(value);
      expect(reasonFor(value)).toBe('insecure_scheme');
    }
    expect(reasonFor('http://api.example.com', true)).toBe('insecure_scheme');
    expect(reasonFor('http://localhost.example.com', true)).toBe('insecure_scheme');
    expect(reasonFor('http://192.168.1.10', true)).toBe('insecure_scheme');
  });

  it('rejects anything beyond an origin', () => {
    for (const value of [
      'https://api.example.com/api',
      'https://api.example.com//',
      'https://api.example.com/?page=1',
      'https://api.example.com/#top',
      'https://user:secret@api.example.com',
    ]) {
      expect(reasonFor(value)).toBe('not_an_origin');
    }
  });

  it('rejects values that are not absolute web URLs', () => {
    expect(reasonFor('')).toBe('invalid_url');
    expect(reasonFor('api.example.com')).toBe('invalid_url');
    expect(reasonFor('ftp://api.example.com')).toBe('insecure_scheme');
  });

  it('names the setting in the message but never the value', () => {
    const error = (() => {
      try {
        parseApiOrigin('https://user:secret@api.example.com', { label: 'PRODUCT_API_URL' });
      } catch (cause) {
        return cause;
      }
      return undefined;
    })();

    expect(error).toBeInstanceOf(TypeError);
    expect((error as Error).message).toBe(
      'PRODUCT_API_URL must be an origin without credentials, path, query, or fragment.',
    );
    expect((error as Error).message).not.toContain('secret');
  });
});

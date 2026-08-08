import { describe, expect, it } from 'vitest';

import { REDACTED_VALUE, scrubProperties } from './scrubber.js';

describe('scrubProperties', () => {
  it('redacts built-in blocked keys at every nesting level', () => {
    const input = {
      email: 'person@example.com',
      displayName: 'Ada',
      nested: {
        auth_token: 'short-token',
        phoneNumber: '+49 123',
      },
      items: [{ shipping_address: 'Main Street' }],
      safe_count: 3,
    };

    expect(scrubProperties(input)).toEqual({
      email: REDACTED_VALUE,
      displayName: REDACTED_VALUE,
      nested: {
        auth_token: REDACTED_VALUE,
        phoneNumber: REDACTED_VALUE,
      },
      items: [{ shipping_address: REDACTED_VALUE }],
      safe_count: 3,
    });
    expect(input.email).toBe('person@example.com');
  });

  it('redacts email-shaped, JWT-shaped, long hex, and long base64 values', () => {
    const scrubbed = scrubProperties({
      contact: 'Please contact person@example.com today',
      credential: 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123',
      hexadecimal: '0123456789abcdef0123456789abcdef',
      encoded: 'VGhpcy1pc19hX3ZlcnlfbG9uZ19zZWNyZXQ',
      ordinary: 'onboarding_completed',
    });

    expect(scrubbed).toEqual({
      contact: REDACTED_VALUE,
      credential: REDACTED_VALUE,
      hexadecimal: REDACTED_VALUE,
      encoded: REDACTED_VALUE,
      ordinary: 'onboarding_completed',
    });
  });

  it('supports product-specific blocked-key extensions', () => {
    expect(
      scrubProperties(
        {
          patientIdentifier: 'short-but-sensitive',
          safe: true,
        },
        { blockedKeys: ['patient_identifier'] },
      ),
    ).toEqual({ patientIdentifier: REDACTED_VALUE, safe: true });
  });

  it('fails closed for cycles and non-serializable object values', () => {
    const cyclic: Record<string, unknown> = {};
    cyclic['self'] = cyclic;

    expect(scrubProperties({ cyclic, date: new Date(0) })).toEqual({
      cyclic: { self: REDACTED_VALUE },
      date: REDACTED_VALUE,
    });
  });
});

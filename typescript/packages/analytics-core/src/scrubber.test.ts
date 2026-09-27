import { describe, expect, it } from 'vitest';

import { REDACTED_VALUE, scrubErrorEvent, scrubProperties } from './scrubber.js';

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

  it('redacts exact keys without matching longer keys that contain them', () => {
    expect(
      scrubProperties(
        {
          ip: '203.0.113.7',
          ipAddress: '203.0.113.7',
          'X-Forwarded-For': '203.0.113.7',
          zip_code_count: 1,
          description_length: 12,
          q: 'free text',
          query_count: 2,
        },
        { exactBlockedKeys: ['q'] },
      ),
    ).toEqual({
      ip: REDACTED_VALUE,
      ipAddress: REDACTED_VALUE,
      'X-Forwarded-For': REDACTED_VALUE,
      zip_code_count: 1,
      description_length: 12,
      q: REDACTED_VALUE,
      query_count: 2,
    });
  });
});

describe('scrubErrorEvent', () => {
  const eventId = '0123456789abcdef0123456789abcdef';
  const traceId = 'fedcba9876543210fedcba9876543210';

  function crashEvent() {
    return {
      event_id: eventId,
      message: 'Sync failed for person@example.com',
      contexts: { trace: { trace_id: traceId, span_id: '0123456789abcdef' } },
      exception: {
        values: [
          {
            type: 'TypeError',
            value: 'x is undefined',
            stacktrace: {
              frames: [
                {
                  filename: 'app:///index.bundle',
                  function: 'renderScreen',
                  lineno: 12,
                  vars: { password: 'secret' },
                },
              ],
            },
          },
        ],
      },
      request: {
        url: 'https://api.example.test/items',
        headers: { Authorization: 'Bearer abc' },
        cookies: 'session=abc',
        query_string: 'q=secret',
        data: { note: 'free text' },
      },
      user: { id: 'user-1', email: 'person@example.com', ip_address: '203.0.113.7' },
      breadcrumbs: [{ category: 'fetch', data: { url: '/items?q=secret' } }],
      sdk: { name: 'sentry.javascript.react-native', packages: [{ name: 'npm:@sentry/core' }] },
      extra: { accessToken: 'abc', attempt: 2 },
    };
  }

  it('keeps crash identifiers and stack frames while redacting request payloads', () => {
    const input = crashEvent();

    expect(scrubErrorEvent(input)).toEqual({
      event_id: eventId,
      message: REDACTED_VALUE,
      contexts: { trace: { trace_id: traceId, span_id: '0123456789abcdef' } },
      exception: {
        values: [
          {
            type: 'TypeError',
            value: 'x is undefined',
            stacktrace: {
              frames: [
                {
                  filename: 'app:///index.bundle',
                  function: 'renderScreen',
                  lineno: 12,
                  vars: REDACTED_VALUE,
                },
              ],
            },
          },
        ],
      },
      request: {
        url: 'https://api.example.test/items',
        headers: REDACTED_VALUE,
        cookies: REDACTED_VALUE,
        query_string: REDACTED_VALUE,
        data: REDACTED_VALUE,
      },
      user: { id: 'user-1', email: REDACTED_VALUE, ip_address: REDACTED_VALUE },
      breadcrumbs: [{ category: 'fetch', data: REDACTED_VALUE }],
      sdk: { name: 'sentry.javascript.react-native', packages: [{ name: 'npm:@sentry/core' }] },
      extra: { accessToken: REDACTED_VALUE, attempt: 2 },
    });
    expect(input).toEqual(crashEvent());
  });

  it('still redacts non-string values under preserved keys', () => {
    expect(scrubErrorEvent({ function: { password: 'secret' } })).toEqual({
      function: { password: REDACTED_VALUE },
    });
  });

  it('applies product key extensions', () => {
    expect(
      scrubErrorEvent(
        { extra: { mealNotes: 'text', q: 'text' } },
        {
          blockedKeys: ['notes'],
          exactBlockedKeys: ['q'],
        },
      ),
    ).toEqual({ extra: { mealNotes: REDACTED_VALUE, q: REDACTED_VALUE } });
  });
});

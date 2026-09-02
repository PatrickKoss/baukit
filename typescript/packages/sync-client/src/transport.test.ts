import { describe, expect, it, vi } from 'vitest';

import {
  SyncAuthError,
  SyncLocalApplyError,
  SyncNetworkError,
  SyncPartitionMismatchError,
  SyncPayloadCompatibilityError,
  SyncRateLimitError,
  SyncServerError,
  SyncTransportError,
  syncFailureFromError,
} from './error.js';
import {
  commitCursorAfterLocalTransaction,
  parseRetryAfter,
  SyncTransport,
  validatePullPage,
  type SyncFetch,
  type SyncFetchResponse,
  type SyncPrebuiltRequest,
} from './transport.js';

function response(status: number, body: string, retryAfter?: string): SyncFetchResponse {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: {
      get: (name) => (name.toLowerCase() === 'retry-after' ? (retryAfter ?? null) : null),
    },
    text: () => Promise.resolve(body),
  };
}

function transport(fetch: SyncFetch, overrides: { partitionHeader?: string } = {}): SyncTransport {
  return new SyncTransport({
    baseUrl: 'https://api.example.test/',
    fetch,
    authHeaders: () => ({ Authorization: 'Bearer token' }),
    ...overrides,
  });
}

describe('SyncTransport', () => {
  it('sends auth headers, the partition header, and a JSON body', async () => {
    const fetch = vi.fn<SyncFetch>(() => Promise.resolve(response(200, '{"accepted":1}')));

    const result = await transport(fetch).request<{ accepted: number }>('/sync/push', {
      method: 'POST',
      body: { changes: [] },
      partitionId: 'partition-1',
    });

    expect(result).toEqual({ accepted: 1 });
    expect(fetch).toHaveBeenCalledWith('https://api.example.test/sync/push', {
      method: 'POST',
      headers: {
        Authorization: 'Bearer token',
        'Content-Type': 'application/json',
        'X-Partition-Id': 'partition-1',
      },
      body: '{"changes":[]}',
    });
  });

  it('awaits an async auth header provider', async () => {
    const fetch = vi.fn<SyncFetch>(() => Promise.resolve(response(200, '{}')));
    const client = new SyncTransport({
      baseUrl: 'https://api.example.test',
      fetch,
      authHeaders: () => Promise.resolve({ Authorization: 'Bearer fresh' }),
    });

    await client.request('/sync/pull');

    expect(fetch.mock.calls[0]?.[1]?.headers).toEqual({ Authorization: 'Bearer fresh' });
  });

  it('delegates a relative request to a prebuilt product API client', async () => {
    const request = vi.fn(() => Promise.resolve({ accepted: 1 }));
    const client = new SyncTransport({ request: request as SyncPrebuiltRequest });

    const result = await client.request<{ accepted: number }>('/sync/push', {
      method: 'POST',
      query: { limit: '50' },
      body: { changes: [] },
      partitionId: 'partition-1',
    });

    expect(result).toEqual({ accepted: 1 });
    expect(request).toHaveBeenCalledWith('/sync/push?limit=50', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Partition-Id': 'partition-1',
      },
      body: '{"changes":[]}',
    });
  });

  it('leaves a prebuilt request function in charge of its errors', async () => {
    const failure = new Error('session recovery failed');
    const request: SyncPrebuiltRequest = () => Promise.reject(failure);
    const client = new SyncTransport({ request });

    await expect(client.request('/sync/pull')).rejects.toBe(failure);
  });

  it('appends query parameters and honours a custom partition header', async () => {
    const fetch = vi.fn<SyncFetch>(() => Promise.resolve(response(200, '{}')));

    await transport(fetch, { partitionHeader: 'X-Profile' }).request('/sync/pull', {
      query: { since_revision: '12', limit: '500' },
      partitionId: 'profile-9',
    });

    expect(fetch.mock.calls[0]?.[0]).toBe(
      'https://api.example.test/sync/pull?since_revision=12&limit=500',
    );
    expect(fetch.mock.calls[0]?.[1]?.headers).toMatchObject({ 'X-Profile': 'profile-9' });
  });

  it('omits the partition header when no partition is supplied', async () => {
    const fetch = vi.fn<SyncFetch>(() => Promise.resolve(response(200, '{}')));

    await transport(fetch).request('/sync/pull', { partitionId: null });

    expect(fetch.mock.calls[0]?.[1]?.headers).toEqual({ Authorization: 'Bearer token' });
  });

  it('maps 401 to a non-retryable auth error', async () => {
    const fetch: SyncFetch = () =>
      Promise.resolve(response(401, '{"message":"token expired","code":"unauthorized"}'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncAuthError);
    expect((error as SyncAuthError).retryable).toBe(false);
    expect((error as SyncAuthError).message).toBe('token expired');
  });

  it('maps the partition mismatch code ahead of the status', async () => {
    const fetch: SyncFetch = () =>
      Promise.resolve(
        response(
          401,
          '{"code":"partition_identity_mismatch","message":"this database was erased"}',
        ),
      );

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncPartitionMismatchError);
    expect((error as SyncPartitionMismatchError).retryable).toBe(false);
  });

  it.each([408, 429, 500, 503])('treats %i as retryable', async (status) => {
    const fetch: SyncFetch = () => Promise.resolve(response(status, '{}'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncTransportError);
    expect((error as SyncTransportError).retryable).toBe(true);
  });

  it('maps 429 to a typed rate-limit error with the parsed retry time', async () => {
    const now = Date.parse('2026-08-22T10:00:00Z');
    const fetch: SyncFetch = () => Promise.resolve(response(429, '{"message":"slow down"}', '17'));
    const client = new SyncTransport({
      baseUrl: 'https://api.example.test',
      fetch,
      authHeaders: () => ({}),
      now: () => now,
    });

    const error = await client.request('/sync/pull').catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncRateLimitError);
    expect(error).toMatchObject({
      kind: 'rate_limited',
      retryable: true,
      retryAfter: '17',
      retryAt: '2026-08-22T10:00:17.000Z',
    });
    expect(error).not.toBeInstanceOf(SyncNetworkError);
  });

  it.each([400, 404, 409, 422])('treats %i as non-retryable', async (status) => {
    const fetch: SyncFetch = () => Promise.resolve(response(status, '{}'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncTransportError);
    expect(error).toBeInstanceOf(SyncServerError);
    expect((error as SyncTransportError).retryable).toBe(false);
    expect((error as SyncTransportError).message).toBe(
      `sync request failed with status ${String(status)}`,
    );
  });

  it('maps a network failure to a retryable error carrying the cause', async () => {
    const cause = new Error('network down');
    const fetch: SyncFetch = () => Promise.reject(cause);

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncTransportError);
    expect(error).toBeInstanceOf(SyncNetworkError);
    expect((error as SyncTransportError).retryable).toBe(true);
    expect((error as SyncTransportError).message).toBe('network down');
    expect((error as SyncTransportError).cause).toBe(cause);
  });

  it('maps an unparsable success body to a non-retryable error', async () => {
    const fetch: SyncFetch = () => Promise.resolve(response(200, 'not json'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncPayloadCompatibilityError);
    expect(error).toBeInstanceOf(SyncTransportError);
    expect((error as SyncTransportError).retryable).toBe(false);
  });

  it('falls back to a status message when the error body is not JSON', async () => {
    const fetch: SyncFetch = () => Promise.resolve(response(500, '<html>gateway</html>'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect((error as SyncTransportError).message).toBe('sync request failed with status 500');
    expect((error as SyncTransportError).retryable).toBe(true);
  });

  it('accepts an empty success body', async () => {
    const fetch: SyncFetch = () => Promise.resolve(response(204, ''));

    await expect(transport(fetch).request('/sync/ack')).resolves.toBeUndefined();
  });
});

describe('parseRetryAfter', () => {
  const now = Date.parse('2026-08-22T10:00:00Z');
  const fallback = '2026-08-22T10:00:42.000Z';

  it('accepts delta seconds', () => {
    expect(parseRetryAfter('12', { now, fallbackMs: 42_000 })).toBe('2026-08-22T10:00:12.000Z');
  });

  it('accepts an HTTP date', () => {
    expect(parseRetryAfter('Sat, 22 Aug 2026 10:02:00 GMT', { now, fallbackMs: 42_000 })).toBe(
      '2026-08-22T10:02:00.000Z',
    );
  });

  it.each([
    ['a past date', 'Sat, 22 Aug 2026 09:59:00 GMT'],
    ['a negative delta', '-5'],
    ['garbage', 'later'],
    ['a missing header', null],
  ])('uses the injected fallback for %s', (_label, value) => {
    expect(parseRetryAfter(value, { now, fallbackMs: 42_000 })).toBe(fallback);
  });
});

describe('syncFailureFromError', () => {
  it('keeps every failure category distinct', () => {
    const retryAt = '2026-08-22T10:02:00.000Z';

    expect([
      syncFailureFromError(new SyncAuthError('auth')),
      syncFailureFromError(new SyncPartitionMismatchError('partition')),
      syncFailureFromError(new SyncRateLimitError('limited', retryAt, '120')),
      syncFailureFromError(new SyncNetworkError('offline')),
      syncFailureFromError(new SyncServerError('server', true)),
      syncFailureFromError(new SyncPayloadCompatibilityError('payload')),
      syncFailureFromError(new SyncLocalApplyError('apply')),
    ]).toEqual([
      { kind: 'auth' },
      { kind: 'partition_mismatch' },
      { kind: 'rate_limited', retryAt },
      { kind: 'network' },
      { kind: 'server' },
      { kind: 'payload_compatibility' },
      { kind: 'local_apply' },
    ]);
  });
});

describe('validatePullPage', () => {
  const compare = (left: number, right: number): number => left - right;

  it('returns a progressing page', () => {
    const page = { nextCursor: 4, hasMore: true, changes: ['row'] };

    expect(validatePullPage(3, page, compare)).toBe(page);
  });

  it('rejects a regressing cursor', () => {
    expect(() => validatePullPage(3, { nextCursor: 2, hasMore: false }, compare)).toThrow(
      SyncPayloadCompatibilityError,
    );
  });

  it('rejects hasMore without progress instead of allowing another loop', () => {
    expect(() => validatePullPage(3, { nextCursor: 3, hasMore: true }, compare)).toThrow(
      SyncPayloadCompatibilityError,
    );
  });
});

describe('commitCursorAfterLocalTransaction', () => {
  it('commits the cursor after the local transaction resolves', async () => {
    const calls: string[] = [];

    const result = await commitCursorAfterLocalTransaction({
      nextCursor: 8,
      transaction: () => {
        calls.push('transaction');
        return Promise.resolve('applied');
      },
      commitCursor: (cursor) => {
        calls.push(`cursor:${String(cursor)}`);
      },
    });

    expect(result).toBe('applied');
    expect(calls).toEqual(['transaction', 'cursor:8']);
  });

  it('leaves the cursor unchanged when the local transaction fails', async () => {
    let cursor = 3;

    const task = commitCursorAfterLocalTransaction({
      nextCursor: 8,
      transaction: () => Promise.reject(new Error('constraint failed')),
      commitCursor: (nextCursor) => {
        cursor = nextCursor;
      },
    });

    await expect(task).rejects.toBeInstanceOf(SyncLocalApplyError);
    expect(cursor).toBe(3);
  });
});

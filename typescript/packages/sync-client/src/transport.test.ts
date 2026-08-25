import { describe, expect, it, vi } from 'vitest';

import { SyncAuthError, SyncPartitionMismatchError, SyncTransportError } from './error.js';
import {
  SyncTransport,
  type SyncFetch,
  type SyncFetchResponse,
  type SyncPrebuiltRequest,
} from './transport.js';

function response(status: number, body: string): SyncFetchResponse {
  return {
    ok: status >= 200 && status < 300,
    status,
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

  it.each([400, 404, 409, 422])('treats %i as non-retryable', async (status) => {
    const fetch: SyncFetch = () => Promise.resolve(response(status, '{}'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(SyncTransportError);
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
    expect((error as SyncTransportError).retryable).toBe(true);
    expect((error as SyncTransportError).message).toBe('network down');
    expect((error as SyncTransportError).cause).toBe(cause);
  });

  it('maps an unparsable success body to a non-retryable error', async () => {
    const fetch: SyncFetch = () => Promise.resolve(response(200, 'not json'));

    const error = await transport(fetch)
      .request('/sync/push')
      .catch((caught: unknown) => caught);

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

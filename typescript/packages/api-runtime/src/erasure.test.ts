import { afterEach, describe, expect, it, vi } from 'vitest';

import receiptVectors from '../../../../fixtures/erasure/receipts-v1.json' with { type: 'json' };

import { ApiError, createApiRuntime, MockFetch } from './index.js';
import { type IdempotencyKeyStorage, type StoredIdempotencyKey } from './idempotency.js';
import { AmbiguousProfileErasureError, createProfileErasureClient } from './erasure.js';

const operationId = 'c6417b1e-4092-4ab8-8dce-e16222c160f3';
const key = 'b7e64cbd-4521-4515-b4c7-3ffb1c7a72b7';
const completedAt = '2026-10-03T12:00:00Z';
const pending = { status: 'pending', operationId };
const completed = { status: 'completed', operationId, completedAt };

function storage(): IdempotencyKeyStorage & {
  readonly entries: ReadonlyMap<string, StoredIdempotencyKey>;
} {
  const persisted = new Map<string, StoredIdempotencyKey>();
  return {
    entries: persisted,
    get: (slot) => persisted.get(slot) ?? null,
    set: (slot, value) => {
      persisted.set(slot, value);
    },
    delete: (slot) => {
      persisted.delete(slot);
    },
  };
}

function setup(persisted = storage(), account = 'subject-a') {
  const transport = new MockFetch();
  const runtime = createApiRuntime({
    baseUrl: 'https://api.example.test',
    environment: 'test',
    fetch: transport.fetch,
    retry: false,
  });
  const client = createProfileErasureClient({
    fetch: runtime.fetch,
    account,
    storage: persisted,
    keyFactory: () => key,
  });
  return { client, transport };
}

function errorBody(code: string) {
  return { error: { code, message: 'Request rejected', requestId: 'request-1', details: {} } };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('profile erasure transport', () => {
  it.each(receiptVectors.cases)('matches the shared $name contract', async (vector) => {
    const persisted = storage();
    const { client, transport } = setup(persisted);
    transport.enqueueJson(vector.receipt, { status: vector.httpStatus });
    if (vector.clientOutcome === 'failed') {
      await expect(client.erase()).rejects.toMatchObject({
        name: 'ProfileErasureOperationFailedError',
        code: 'erasure_operation_failed',
        operationId: vector.receipt.operationId,
      });
      expect(persisted.entries.size).toBe(1);
      transport.enqueueJson(completed, { status: 200 });
      await expect(client.erase()).resolves.toEqual(completed);
      expect(transport.request(1).headers.get('Idempotency-Key')).toBe(key);
    } else {
      await expect(client.erase()).resolves.toEqual(vector.receipt);
    }
    expect(persisted.entries.size).toBe(0);
  });
  it.each([
    [200, completed],
    [202, pending],
  ] as const)('parses a %i receipt and sends a persisted key', async (status, body) => {
    const persisted = storage();
    const set = vi.spyOn(persisted, 'set');
    const { client, transport } = setup(persisted);
    transport.enqueue((request) => {
      expect(set).toHaveBeenCalledOnce();
      expect(request.headers.get('Idempotency-Key')).toBe(key);
      return Response.json(body, { status });
    });
    await expect(client.erase()).resolves.toEqual(body);
    expect(persisted.entries.size).toBe(0);
    transport.assertRequest(0, { method: 'DELETE', url: 'https://api.example.test/me' });
  });

  it('replays the persisted key in a new client after a dropped response', async () => {
    const persisted = storage();
    const first = setup(persisted);
    first.transport.enqueue(new TypeError('Connection dropped after commit'));
    await expect(first.client.erase()).rejects.toBeInstanceOf(AmbiguousProfileErasureError);

    expect(persisted.entries.size).toBe(1);
    const second = setup(persisted);
    second.transport.enqueueJson(pending, { status: 202 });
    await expect(second.client.erase()).resolves.toEqual(pending);
    expect(persisted.entries.size).toBe(0);
    expect(second.transport.request(0).headers.get('Idempotency-Key')).toBe(
      first.transport.request(0).headers.get('Idempotency-Key'),
    );
  });

  it.each([
    [200, completed],
    [202, pending],
  ] as const)('preserves the committed %i receipt if key removal fails', async (status, body) => {
    const persisted = storage();
    vi.spyOn(persisted, 'delete').mockRejectedValue(new Error('Storage unavailable'));
    const { client, transport } = setup(persisted);
    transport.enqueueJson(body, { status });
    await expect(client.erase()).rejects.toMatchObject({
      name: 'ProfileErasureKeyCleanupError',
      receipt: body,
    });
    expect(persisted.entries.size).toBe(1);
  });

  it('reports 409 as a definite rejection and releases its key', async () => {
    const persisted = storage();
    const remove = vi.spyOn(persisted, 'delete');
    const { client, transport } = setup(persisted);
    transport.enqueueJson(errorBody('erasure_idempotency_conflict'), { status: 409 });
    await expect(client.erase()).rejects.toMatchObject({
      status: 409,
      code: 'erasure_idempotency_conflict',
    });
    expect(remove).toHaveBeenCalledOnce();
  });

  it('accepts a 401 profile_erased as committed with identity deletion still unconfirmed', async () => {
    const persisted = storage();
    const { client, transport } = setup(persisted);
    transport.enqueueJson(errorBody('profile_erased'), { status: 401 });
    await expect(client.erase()).resolves.toEqual({ status: 'pending', operationId: null });
    expect(persisted.entries.size).toBe(0);
  });

  it('retains the original key if a later retry is unauthorized after a lost response', async () => {
    const persisted = storage();
    const remove = vi.spyOn(persisted, 'delete');
    const first = setup(persisted);
    first.transport.enqueue(new TypeError('Response lost after commit'));
    await expect(first.client.erase()).rejects.toBeInstanceOf(AmbiguousProfileErasureError);
    const second = setup(persisted);
    second.transport.enqueueJson(errorBody('unauthorized'), { status: 401 });
    await expect(second.client.erase()).rejects.toMatchObject({ status: 401 });
    expect(remove).not.toHaveBeenCalled();
    const third = setup(persisted);
    third.transport.enqueueJson(pending, { status: 202 });
    await expect(third.client.erase()).resolves.toEqual(pending);
    expect(third.transport.request(0).headers.get('Idempotency-Key')).toBe(
      first.transport.request(0).headers.get('Idempotency-Key'),
    );
  });

  it('also handles a raw fetch that returns the fence response', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(Response.json(errorBody('profile_erased'), { status: 401 })),
    );
    const client = createProfileErasureClient({
      fetch,
      account: 'account',
      storage: storage(),
      keyFactory: () => key,
    });
    await expect(client.erase()).resolves.toEqual({ status: 'pending', operationId: null });
  });

  it('keeps an ordinary 401 as a rejection', async () => {
    const { client, transport } = setup();
    transport.enqueueJson(errorBody('unauthorized'), { status: 401 });
    await expect(client.erase()).rejects.toBeInstanceOf(ApiError);
  });

  it.each([
    [200, pending],
    [202, completed],
    [200, { ...completed, completedAt: 'bad' }],
    [200, { ...completed, operationId: 'not-a-uuid' }],
    [200, { status: 'completed', operationId }],
    [202, { ...pending, status: 'failed' }],
    [204, null],
    [200, null],
  ])('keeps malformed %i replies ambiguous', async (status, body) => {
    const persisted = storage();
    const remove = vi.spyOn(persisted, 'delete');
    const { client, transport } = setup(persisted);
    transport.enqueue(
      status === 204 ? new Response(null, { status }) : Response.json(body, { status }),
    );
    await expect(client.erase()).rejects.toMatchObject({
      code: 'product_profile_erasure_ambiguous',
    });
    expect(remove).not.toHaveBeenCalled();
  });

  it('does not send if durable key storage fails', async () => {
    const persisted = storage();
    vi.spyOn(persisted, 'set').mockRejectedValue(new Error('Disk full'));
    const { client, transport } = setup(persisted);
    await expect(client.erase()).rejects.toThrow('Disk full');
    expect(transport.requests).toHaveLength(0);
  });

  it('separates key storage slots by account', async () => {
    const persisted = storage();
    const set = vi.spyOn(persisted, 'set');
    const first = setup(persisted, 'subject-a');
    const second = setup(persisted, 'subject-b');
    first.transport.enqueueJson(pending, { status: 202 });
    second.transport.enqueueJson(pending, { status: 202 });
    await first.client.erase();
    await second.client.erase();
    expect(set).toHaveBeenCalledTimes(2);
    expect(set.mock.calls[0]?.[0]).not.toBe(set.mock.calls[1]?.[0]);
  });

  it('rejects an invalid key before sending', async () => {
    const fetch = vi.fn();
    const client = createProfileErasureClient({
      fetch,
      account: 'account',
      storage: storage(),
      keyFactory: () => 'short',
    });
    await expect(client.erase()).rejects.toThrow('Invalid erasure idempotency key.');
    expect(fetch).not.toHaveBeenCalled();
  });
});

describe('profile erasure polling', () => {
  it.each(['completed', 'failed'] as const)(
    'polls to %s with capped exponential backoff',
    async (status) => {
      vi.useFakeTimers();
      const { client, transport } = setup();
      for (let index = 0; index < 4; index += 1) transport.enqueueJson(pending);
      const terminal = status === 'completed' ? completed : { status, operationId };
      transport.enqueueJson(terminal);
      const polling = client.poll(operationId, { initialDelayMs: 100, maxDelayMs: 250 });
      await vi.advanceTimersByTimeAsync(0);
      expect(transport.requests).toHaveLength(1);
      for (const [index, delay] of [100, 200, 250, 250].entries()) {
        await vi.advanceTimersByTimeAsync(delay - 1);
        expect(transport.requests).toHaveLength(index + 1);
        await vi.advanceTimersByTimeAsync(1);
        expect(transport.requests).toHaveLength(index + 2);
      }
      await expect(polling).resolves.toEqual(terminal);
      transport.assertRequest(4, {
        method: 'GET',
        url: `https://api.example.test/me/erasures/${operationId}`,
      });
      expect(vi.getTimerCount()).toBe(0);
    },
  );

  it('returns pending at the polling limit', async () => {
    vi.useFakeTimers();
    const { client, transport } = setup();
    transport.enqueueJson(pending).enqueueJson(pending);
    const polling = client.poll(operationId, { maxAttempts: 2 });
    await vi.runAllTimersAsync();
    await expect(polling).resolves.toEqual(pending);
    expect(transport.requests).toHaveLength(2);
  });

  it('aborts a backoff delay without sending another request', async () => {
    vi.useFakeTimers();
    const { client, transport } = setup();
    transport.enqueueJson(pending);
    const controller = new AbortController();
    const polling = client.poll(operationId, { signal: controller.signal });
    const rejected = expect(polling).rejects.toMatchObject({ name: 'AbortError' });
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    await rejected;
    expect(transport.requests).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('aborts before sending and forwards the signal to in-flight reads', async () => {
    const { client, transport } = setup();
    const controller = new AbortController();
    transport.enqueue((request) => {
      expect(request.signal.aborted).toBe(false);
      controller.abort();
      return Response.json(pending);
    });
    await expect(client.poll(operationId, { signal: controller.signal })).rejects.toMatchObject({
      aborted: true,
    });
    await expect(client.poll(operationId, { signal: controller.signal })).rejects.toMatchObject({
      name: 'AbortError',
    });
    expect(transport.requests).toHaveLength(1);
  });

  it('keeps profile_erased during polling pending rather than guessing that the IdP finished', async () => {
    const { client, transport } = setup();
    transport.enqueueJson(errorBody('profile_erased'), { status: 401 });
    await expect(client.poll(operationId, { maxAttempts: 1 })).resolves.toEqual(pending);
  });

  it('rejects a foreign operation receipt', async () => {
    const { client, transport } = setup();
    transport.enqueueJson({ ...completed, operationId: key });
    await expect(client.poll(operationId)).rejects.toThrow('Erasure operation ID mismatch.');
  });

  it('preserves 404 and ordinary 401 errors', async () => {
    const { client, transport } = setup();
    transport.enqueueJson(errorBody('not_found'), { status: 404 });
    transport.enqueueJson(errorBody('unauthorized'), { status: 401 });
    await expect(client.poll(operationId)).rejects.toMatchObject({ status: 404 });
    await expect(client.poll(operationId)).rejects.toMatchObject({ status: 401 });
  });

  it.each([{ maxAttempts: 0 }, { initialDelayMs: -1 }, { maxDelayMs: Number.NaN }])(
    'rejects invalid polling settings %j',
    async (polling) => {
      const { client, transport } = setup();
      await expect(client.poll(operationId, polling)).rejects.toBeInstanceOf(RangeError);
      expect(transport.requests).toHaveLength(0);
    },
  );
});

import { describe, expect, it } from 'vitest';

import {
  canonicalJson,
  classifyMutationError,
  classifyMutationStatus,
  createIdempotencyKeyStore,
  sendIdempotentMutation,
  type IdempotencyKeyStorage,
  type MutationIntent,
  type StoredIdempotencyKey,
} from './idempotency.js';
import { ApiError, HttpError, NetworkError } from './index.js';

const TTL_MS = 60_000;
const intent: MutationIntent = {
  account: 'account-a',
  operation: 'createNote',
  body: { title: 'standup', tags: ['a', 'b'] },
};

function apiError(status: number, code: string): ApiError {
  return new ApiError(
    { error: { code, message: 'failed', request_id: 'req-1', details: {} } },
    status,
  );
}

function counterKeys(): () => string {
  let next = 0;
  return () => {
    next += 1;
    return `key-${String(next)}`;
  };
}

function store(now: () => number = () => 0, storage?: IdempotencyKeyStorage) {
  return createIdempotencyKeyStore({
    ttlMs: TTL_MS,
    now,
    keyFactory: counterKeys(),
    ...(storage === undefined ? {} : { storage }),
  });
}

describe('classifyMutationStatus', () => {
  it.each([
    [200, 'committed'],
    [201, 'committed'],
    [204, 'committed'],
    [400, 'not-committed'],
    [404, 'not-committed'],
    [409, 'not-committed'],
    [412, 'not-committed'],
    [408, 'possibly-committed'],
    [429, 'possibly-committed'],
    [500, 'possibly-committed'],
    [503, 'possibly-committed'],
    [302, 'possibly-committed'],
  ] as const)('classifies %i as %s', (status, outcome) => {
    expect(classifyMutationStatus(status)).toBe(outcome);
  });
});

describe('classifyMutationError', () => {
  it('treats transport failures and unknown throws as possibly committed', () => {
    expect(classifyMutationError(new NetworkError('offline', 'req-1', null))).toBe(
      'possibly-committed',
    );
    expect(classifyMutationError(new NetworkError('aborted', 'req-1', null, true))).toBe(
      'possibly-committed',
    );
    expect(classifyMutationError(new TypeError('boom'))).toBe('possibly-committed');
  });

  it('classifies API and HTTP errors by status', () => {
    expect(classifyMutationError(apiError(400, 'validation_failed'))).toBe('not-committed');
    expect(classifyMutationError(apiError(409, 'idempotency_key_reused'))).toBe('not-committed');
    expect(classifyMutationError(apiError(503, 'unavailable'))).toBe('possibly-committed');
    expect(classifyMutationError(new HttpError(new Response(null, { status: 502 }), null))).toBe(
      'possibly-committed',
    );
  });

  it('treats a key still in progress as possibly committed', () => {
    expect(classifyMutationError(apiError(409, 'idempotency_key_in_progress'))).toBe(
      'possibly-committed',
    );
  });
});

describe('createIdempotencyKeyStore', () => {
  it('keeps one key per account, operation, and body', async () => {
    const keys = store();
    const first = await keys.keyFor(intent);

    await expect(
      keys.keyFor({ ...intent, body: { tags: ['a', 'b'], title: 'standup' } }),
    ).resolves.toBe(first);
    await expect(keys.keyFor({ ...intent, account: 'account-b' })).resolves.not.toBe(first);
    await expect(keys.keyFor({ ...intent, operation: 'createTask' })).resolves.not.toBe(first);
    await expect(keys.keyFor({ ...intent, body: { title: 'retro' } })).resolves.not.toBe(first);
    await expect(
      keys.keyFor({ ...intent, body: { title: 'standup', tags: ['b', 'a'] } }),
    ).resolves.not.toBe(first);
  });

  it('keeps the key while the outcome is uncertain and drops it after a definite one', async () => {
    const keys = store();
    const first = await keys.keyFor(intent);

    await keys.settle(intent, 'possibly-committed');
    await expect(keys.keyFor(intent)).resolves.toBe(first);

    await keys.settle(intent, 'committed');
    const second = await keys.keyFor(intent);
    expect(second).not.toBe(first);

    await keys.settle(intent, 'not-committed');
    await expect(keys.keyFor(intent)).resolves.not.toBe(second);
  });

  it('replaces a key after its time to live', async () => {
    let now = 0;
    const keys = store(() => now);
    const first = await keys.keyFor(intent);

    now = TTL_MS - 1;
    await expect(keys.keyFor(intent)).resolves.toBe(first);
    now = TTL_MS;
    await expect(keys.keyFor(intent)).resolves.not.toBe(first);
  });

  it('reads keys back from caller storage after a restart', async () => {
    const saved = new Map<string, StoredIdempotencyKey>();
    const storage: IdempotencyKeyStorage = {
      get: (slot) => Promise.resolve(saved.get(slot) ?? null),
      set: (slot, value) => {
        saved.set(slot, value);
        return Promise.resolve();
      },
      delete: (slot) => {
        saved.delete(slot);
        return Promise.resolve();
      },
    };
    const first = await store(() => 0, storage).keyFor(intent);

    await expect(store(() => 1, storage).keyFor(intent)).resolves.toBe(first);
  });

  it('rejects a time to live that is not positive', () => {
    expect(() => createIdempotencyKeyStore({ ttlMs: 0 })).toThrow('ttlMs');
    expect(() => createIdempotencyKeyStore({ ttlMs: Number.NaN })).toThrow('ttlMs');
  });

  it('uses random UUID keys by default', async () => {
    const key = await createIdempotencyKeyStore({ ttlMs: TTL_MS }).keyFor(intent);
    expect(key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
  });
});

describe('sendIdempotentMutation', () => {
  it('resends a lost response with the same key and drops the key after success', async () => {
    const keys = store();
    const sent: string[] = [];

    await expect(
      sendIdempotentMutation(keys, intent, (key) => {
        sent.push(key);
        return Promise.reject(new NetworkError('offline', 'req-1', null));
      }),
    ).rejects.toBeInstanceOf(NetworkError);
    await expect(
      sendIdempotentMutation(keys, intent, (key) => {
        sent.push(key);
        return Promise.resolve('created');
      }),
    ).resolves.toBe('created');
    await sendIdempotentMutation(keys, intent, (key) => {
      sent.push(key);
      return Promise.resolve('created again');
    });

    expect(sent).toEqual(['key-1', 'key-1', 'key-2']);
  });

  it('drops the key after a rejection that did not commit', async () => {
    const keys = store();
    await expect(
      sendIdempotentMutation(keys, intent, () =>
        Promise.reject(apiError(400, 'validation_failed')),
      ),
    ).rejects.toBeInstanceOf(ApiError);

    await expect(keys.keyFor(intent)).resolves.toBe('key-2');
  });
});

describe('canonicalJson', () => {
  it('sorts object members at every depth and keeps array order', () => {
    expect(canonicalJson({ b: [2, { d: 1, c: null }], a: 'x' })).toBe(
      '{"a":"x","b":[2,{"c":null,"d":1}]}',
    );
  });

  it('rejects non-finite numbers', () => {
    expect(() => canonicalJson(Number.POSITIVE_INFINITY)).toThrow('non-finite');
  });
});

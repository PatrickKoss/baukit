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
    { error: { code, message: 'failed', requestId: 'req-1', details: {} } },
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

  it('leaves out undefined members and writes undefined array items as null', () => {
    expect(canonicalJson({ b: undefined, a: [1, undefined] })).toBe('{"a":[1,null]}');
  });

  it('rejects non-finite numbers', () => {
    expect(() => canonicalJson(Number.POSITIVE_INFINITY)).toThrow('non-finite');
  });
});

describe('JSON-compatible bodies', () => {
  interface ChoiceBody {
    readonly text: string;
    readonly audio?: string | null | undefined;
  }

  interface DraftBody {
    readonly title: string;
    readonly choices: readonly ChoiceBody[];
    readonly quality: 'strong' | 'weak';
  }

  it('accepts interface bodies without an index signature', async () => {
    const keys = store();
    const body: DraftBody = { title: 'intro', choices: [{ text: 'hi' }], quality: 'strong' };
    const sent: string[] = [];

    await sendIdempotentMutation(keys, { account: 'a', operation: 'createDraft', body }, (key) => {
      sent.push(key);
      return Promise.resolve();
    });

    expect(sent).toEqual(['key-1']);
  });

  it('gives a body with undefined members the key of the body without them', async () => {
    const keys = store();
    const sparse: DraftBody = {
      title: 'intro',
      choices: [{ text: 'hi', audio: undefined }],
      quality: 'weak',
    };
    const dense: DraftBody = { title: 'intro', choices: [{ text: 'hi' }], quality: 'weak' };

    const first = await keys.keyFor({ account: 'a', operation: 'createDraft', body: sparse });

    await expect(
      keys.keyFor({ account: 'a', operation: 'createDraft', body: dense }),
    ).resolves.toBe(first);
  });

  it('rejects bodies that JSON cannot carry unchanged', () => {
    const keys = store();
    // @ts-expect-error a Date is not a JSON value
    void keys.keyFor({ account: 'a', operation: 'op', body: { at: new Date(0) } });
    // @ts-expect-error a function is not a JSON value
    void keys.keyFor({ account: 'a', operation: 'op', body: { run: () => 1 } });
    // @ts-expect-error an unknown member is not known to be JSON
    void keys.keyFor({ account: 'a', operation: 'op', body: { value: 1 as unknown } });
  });
});

describe('product error classification', () => {
  class ProductApiError extends Error {
    constructor(
      readonly status: number,
      readonly code?: string,
    ) {
      super('product failure');
    }
  }

  function classifyProductError(error: unknown) {
    return error instanceof ProductApiError
      ? classifyMutationStatus(error.status, error.code)
      : undefined;
  }

  it('drops the key when the product classifier reports a definite rejection', async () => {
    const keys = createIdempotencyKeyStore({
      ttlMs: TTL_MS,
      keyFactory: counterKeys(),
      classifyError: classifyProductError,
    });

    await expect(
      sendIdempotentMutation(keys, intent, () => Promise.reject(new ProductApiError(422))),
    ).rejects.toBeInstanceOf(ProductApiError);

    await expect(keys.keyFor(intent)).resolves.toBe('key-2');
  });

  it('falls back to the Baukit classifier for errors the product does not know', () => {
    const keys = createIdempotencyKeyStore({ ttlMs: TTL_MS, classifyError: classifyProductError });

    expect(keys.classifyError(new ProductApiError(409, 'idempotency_key_in_progress'))).toBe(
      'possibly-committed',
    );
    expect(keys.classifyError(new ProductApiError(404))).toBe('not-committed');
    expect(keys.classifyError(apiError(400, 'validation_failed'))).toBe('not-committed');
    expect(keys.classifyError(new TypeError('offline'))).toBe('possibly-committed');
  });

  it('keeps the key for a product error when no classifier is set', async () => {
    const keys = store();

    await expect(
      sendIdempotentMutation(keys, intent, () => Promise.reject(new ProductApiError(422))),
    ).rejects.toBeInstanceOf(ProductApiError);

    await expect(keys.keyFor(intent)).resolves.toBe('key-1');
  });

  it('treats the in-progress code as possibly committed', () => {
    expect(classifyMutationStatus(409, 'idempotency_key_in_progress')).toBe('possibly-committed');
    expect(classifyMutationStatus(409, 'idempotency_key_reused')).toBe('not-committed');
  });
});

describe('concurrent keyFor calls', () => {
  const droppingStorage = (): IdempotencyKeyStorage => ({
    get: () => null,
    set: () => undefined,
    delete: () => undefined,
  });

  function gatedStorage(): IdempotencyKeyStorage & { open(): void } {
    const saved = new Map<string, StoredIdempotencyKey>();
    let opened: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      opened = resolve;
    });
    return {
      async get(slot) {
        await gate;
        return saved.get(slot) ?? null;
      },
      async set(slot, value) {
        await gate;
        saved.set(slot, value);
      },
      delete(slot) {
        saved.delete(slot);
      },
      open: () => {
        opened();
      },
    };
  }

  it('share one key while the first lookup is still reading storage', async () => {
    const storage = gatedStorage();
    const keys = store(() => 0, storage);

    const first = keys.keyFor(intent);
    const second = keys.keyFor(intent);
    storage.open();

    await expect(Promise.all([first, second])).resolves.toEqual(['key-1', 'key-1']);
  });

  it('share one key when storage drops writes, until the intent settles', async () => {
    const keys = store(() => 0, droppingStorage());

    const first = await keys.keyFor(intent);
    await expect(keys.keyFor(intent)).resolves.toBe(first);

    await keys.settle(intent, 'committed');
    await expect(keys.keyFor(intent)).resolves.not.toBe(first);
  });

  it('do not reuse a held key after its time to live', async () => {
    let now = 0;
    const keys = store(() => now, droppingStorage());

    const first = await keys.keyFor(intent);
    now = TTL_MS;

    await expect(keys.keyFor(intent)).resolves.not.toBe(first);
  });

  it('recover when an earlier lookup failed', async () => {
    let failNext = true;
    const flaky: IdempotencyKeyStorage = {
      ...droppingStorage(),
      get: () => {
        if (failNext) {
          failNext = false;
          return Promise.reject(new Error('storage offline'));
        }
        return null;
      },
    };
    const keys = store(() => 0, flaky);

    const failed = keys.keyFor(intent);
    const next = keys.keyFor(intent);

    await expect(failed).rejects.toThrow('storage offline');
    await expect(next).resolves.toBe('key-1');
  });

  it('send two in-flight mutations for one intent with one key', async () => {
    const keys = store();
    const sent: string[] = [];
    const send = (key: string) => {
      sent.push(key);
      return Promise.resolve();
    };

    await Promise.all([
      sendIdempotentMutation(keys, intent, send),
      sendIdempotentMutation(keys, intent, send),
    ]);

    expect(sent).toEqual(['key-1', 'key-1']);
  });
});

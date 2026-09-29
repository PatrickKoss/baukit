import { describe, expect, it } from 'vitest';

import { type JsonValue, type KeyValueStore, StorageError } from './contracts.js';
import {
  type DraftCodec,
  type DraftDecodeResult,
  type DurableDraft,
  type DurableDraftSnapshot,
  DraftPersistenceError,
  createDurableDraft,
} from './durable-draft.js';
import { InMemoryKeyValueStore } from './memory.js';

interface Deferred {
  readonly promise: Promise<void>;
  resolve(): void;
}

function deferred(): Deferred {
  let resolve!: () => void;
  const promise = new Promise<void>((onResolve) => {
    resolve = onResolve;
  });
  return { promise, resolve };
}

type Operation = 'get' | 'set' | 'delete';

/** An in-memory store whose operations can be held open or made to fail. */
class ControlledStore implements KeyValueStore {
  readonly inner = new InMemoryKeyValueStore();
  readonly calls: { readonly operation: Operation; readonly key: string }[] = [];
  readonly #gates = new Map<Operation, Deferred[]>();
  readonly #failures = new Map<Operation, Error[]>();

  hold(operation: Operation): Deferred {
    const gate = deferred();
    const gates = this.#gates.get(operation) ?? [];
    gates.push(gate);
    this.#gates.set(operation, gates);
    return gate;
  }

  failNext(operation: Operation, error: Error): void {
    const failures = this.#failures.get(operation) ?? [];
    failures.push(error);
    this.#failures.set(operation, failures);
  }

  async get(key: string): Promise<JsonValue | undefined> {
    await this.#enter('get', key);
    return this.inner.get(key);
  }

  async set(key: string, value: JsonValue): Promise<void> {
    await this.#enter('set', key);
    await this.inner.set(key, value);
  }

  async delete(key: string): Promise<void> {
    await this.#enter('delete', key);
    await this.inner.delete(key);
  }

  clear(): Promise<void> {
    return this.inner.clear();
  }

  clearPrefix(prefix: string): Promise<void> {
    return this.inner.clearPrefix(prefix);
  }

  async #enter(operation: Operation, key: string): Promise<void> {
    this.calls.push({ operation, key });
    const gate = this.#gates.get(operation)?.shift();
    if (gate !== undefined) {
      await gate.promise;
    }
    const failure = this.#failures.get(operation)?.shift();
    if (failure !== undefined) {
      throw failure;
    }
  }
}

interface Scope {
  readonly account: string;
  readonly document: string;
}

interface FormValue {
  readonly title: string;
  readonly tags: readonly string[];
}

const EMPTY: FormValue = { title: '', tags: [] };
const accountA: Scope = { account: 'account-a', document: 'note-1' };
const accountB: Scope = { account: 'account-b', document: 'note-1' };
const otherDocument: Scope = { account: 'account-a', document: 'note-2' };

function isRecord(value: JsonValue): value is Record<string, JsonValue> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isStringArray(value: JsonValue | undefined): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function decodeFormValue(value: JsonValue): DraftDecodeResult<FormValue> {
  if (!isRecord(value) || typeof value['title'] !== 'string' || !isStringArray(value['tags'])) {
    return { kind: 'corrupt' };
  }
  return { kind: 'decoded', value: { title: value['title'], tags: value['tags'] } };
}

/** Version 1 stored a single `tag`; version 2 stores `tags`. */
const codecV2: DraftCodec<FormValue> = {
  version: 2,
  encode: (value) => ({ title: value.title, tags: [...value.tags] }),
  decode(value, version) {
    if (version === 2) {
      return decodeFormValue(value);
    }
    if (!isRecord(value) || typeof value['title'] !== 'string') {
      return { kind: 'corrupt' };
    }
    const tag = value['tag'];
    return {
      kind: 'decoded',
      value: { title: value['title'], tags: typeof tag === 'string' ? [tag] : [] },
    };
  },
};

const codecV1: DraftCodec<FormValue> = {
  version: 1,
  encode: (value) => ({ title: value.title, tags: [...value.tags] }),
  decode: (value) => decodeFormValue(value),
};

function draftKey(scope: Scope): string {
  return `draft:${encodeURIComponent(scope.account)}:${encodeURIComponent(scope.document)}`;
}

function createDraft(
  store: KeyValueStore,
  codec: DraftCodec<FormValue> = codecV2,
): DurableDraft<Scope, FormValue> {
  return createDurableDraft({ store, key: draftKey, codec });
}

type OpenSnapshot = Extract<DurableDraftSnapshot<Scope, FormValue>, { readonly open: true }>;

function openSnapshot(draft: DurableDraft<Scope, FormValue>): OpenSnapshot {
  const snapshot = draft.getSnapshot();
  if (!snapshot.open) {
    throw new Error('Draft is closed.');
  }
  return snapshot;
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 10; turn += 1) {
    await Promise.resolve();
  }
}

describe('durable draft', () => {
  describe('opening', () => {
    it('starts closed', () => {
      const draft = createDraft(new ControlledStore());
      expect(draft.getSnapshot()).toEqual({ open: false });
    });

    it('reports an empty draft with the initial value when nothing is stored', async () => {
      const draft = createDraft(new ControlledStore());

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft)).toMatchObject({
        scope: accountA,
        value: EMPTY,
        recovery: 'none',
        persistence: 'idle',
        dirty: false,
        error: null,
        submission: 'none',
      });
    });

    it('restores a stored draft of the current version', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 2, value: { title: 'Kept', tags: [] } });
      const draft = createDraft(store);

      const opened = draft.open(accountA, EMPTY);
      expect(openSnapshot(draft).persistence).toBe('loading');
      await opened;

      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Kept', tags: [] },
        recovery: 'restored',
        persistence: 'idle',
        dirty: false,
      });
    });

    it('reads the existing version 1 envelope written as { version, value }', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), {
        version: 1,
        value: { title: 'Legacy', tags: ['a'] },
      });
      const draft = createDraft(store, codecV1);

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Legacy', tags: ['a'] },
        recovery: 'restored',
        dirty: false,
      });
    });

    it('keeps an edit made while the stored draft was loading', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 2, value: { title: 'Old', tags: [] } });
      const gate = store.hold('get');
      const draft = createDraft(store);

      const opened = draft.open(accountA, EMPTY);
      draft.update({ title: 'Typed', tags: [] });
      gate.resolve();
      await opened;

      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Typed', tags: [] },
        recovery: 'none',
        dirty: true,
      });
    });
  });

  describe('corrupt drafts', () => {
    const corruptEnvelopes: readonly (readonly [string, JsonValue])[] = [
      ['a bare string', 'not a draft'],
      ['an array', [1, 2]],
      ['a missing value', { version: 2 }],
      ['a non-integer version', { version: 1.5, value: {} }],
      ['a zero version', { version: 0, value: {} }],
      ['a string version', { version: '2', value: {} }],
      ['a value the codec rejects', { version: 2, value: { title: 7 } }],
    ];

    it.each(corruptEnvelopes)(
      'reports %s as corrupt without touching storage',
      async (_, stored) => {
        const store = new ControlledStore();
        await store.inner.set(draftKey(accountA), stored);
        const draft = createDraft(store);

        await draft.open(accountA, EMPTY);

        expect(openSnapshot(draft)).toMatchObject({
          value: EMPTY,
          recovery: 'corrupt',
          persistence: 'idle',
          error: null,
        });
        expect(store.calls.map((call) => call.operation)).toEqual(['get']);
        await expect(store.inner.get(draftKey(accountA))).resolves.toEqual(stored);
      },
    );

    it('reports a throwing decoder as corrupt', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 2, value: {} });
      const draft = createDurableDraft<Scope, FormValue>({
        store,
        key: draftKey,
        codec: {
          version: 2,
          encode: (value) => codecV2.encode(value),
          decode: () => {
            throw new Error('secret decoder text');
          },
        },
      });

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft)).toMatchObject({ recovery: 'corrupt', error: null });
    });

    it('blocks saves until the product discards the corrupt draft', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), 'garbage');
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);

      draft.update({ title: 'New', tags: [] });
      await expect(draft.save()).resolves.toBe('blocked');
      await expect(store.inner.get(draftKey(accountA))).resolves.toBe('garbage');

      await expect(draft.clear({ reason: 'discarded' })).resolves.toBe('cleared');
      expect(openSnapshot(draft)).toMatchObject({ recovery: 'none', value: EMPTY, dirty: false });

      draft.update({ title: 'New', tags: [] });
      await expect(draft.save()).resolves.toBe('saved');
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'New', tags: [] },
      });
    });
  });

  describe('codec versions', () => {
    it('upgrades an older version and marks it dirty for the next save', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 1, value: { title: 'Old', tag: 'x' } });
      const draft = createDraft(store);

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Old', tags: ['x'] },
        recovery: 'restored',
        dirty: true,
      });
      expect(store.calls.map((call) => call.operation)).toEqual(['get']);

      await expect(draft.save()).resolves.toBe('saved');
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'Old', tags: ['x'] },
      });
      expect(openSnapshot(draft).dirty).toBe(false);
    });

    it('reports a newer version as unsupported without decoding, writing, or deleting', async () => {
      const store = new ControlledStore();
      const stored = { version: 3, value: { title: 'Future' } };
      await store.inner.set(draftKey(accountA), stored);
      let decodeCalls = 0;
      const draft = createDurableDraft<Scope, FormValue>({
        store,
        key: draftKey,
        codec: {
          ...codecV2,
          decode: (value, version) => {
            decodeCalls += 1;
            return codecV2.decode(value, version);
          },
        },
      });

      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Mine', tags: [] });
      await expect(draft.save()).resolves.toBe('blocked');

      expect(decodeCalls).toBe(0);
      expect(openSnapshot(draft)).toMatchObject({
        recovery: 'unsupported-version',
        value: { title: 'Mine', tags: [] },
        dirty: true,
      });
      expect(store.calls.map((call) => call.operation)).toEqual(['get']);
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual(stored);
    });

    it('lets the codec refuse an older version it no longer reads', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 1, value: {} });
      const draft = createDurableDraft<Scope, FormValue>({
        store,
        key: draftKey,
        codec: {
          ...codecV2,
          decode: (value, version) =>
            version === 2 ? codecV2.decode(value, version) : { kind: 'unsupported-version' },
        },
      });

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft).recovery).toBe('unsupported-version');
    });
  });

  describe('saving', () => {
    it('moves from dirty to saving to clean', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);

      draft.update({ title: 'A', tags: [] });
      expect(openSnapshot(draft)).toMatchObject({ dirty: true, persistence: 'idle' });
      const gate = store.hold('set');
      const saved = draft.save();
      await settle();
      expect(openSnapshot(draft).persistence).toBe('saving');
      gate.resolve();

      await expect(saved).resolves.toBe('saved');
      expect(openSnapshot(draft)).toMatchObject({ dirty: false, persistence: 'idle' });
    });

    it('keeps an edit made during a save dirty', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);

      draft.update({ title: 'A', tags: [] });
      const gate = store.hold('set');
      const saved = draft.save();
      await settle();
      draft.update({ title: 'AB', tags: [] });
      gate.resolve();
      await saved;

      expect(openSnapshot(draft)).toMatchObject({ value: { title: 'AB', tags: [] }, dirty: true });
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'A', tags: [] },
      });
    });

    it('writes only the newest value when saves queue up', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);

      draft.update({ title: 'A', tags: [] });
      const gate = store.hold('set');
      const first = draft.save();
      await settle();
      draft.update({ title: 'AB', tags: [] });
      const second = draft.save();
      draft.update({ title: 'ABC', tags: [] });
      const third = draft.save();
      gate.resolve();

      await expect(Promise.all([first, second, third])).resolves.toEqual([
        'saved',
        'saved',
        'clean',
      ]);
      expect(store.calls.filter((call) => call.operation === 'set')).toHaveLength(2);
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'ABC', tags: [] },
      });
    });

    it('resolves clean when nothing changed', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);

      await expect(draft.save()).resolves.toBe('clean');
      expect(store.calls.map((call) => call.operation)).toEqual(['get']);
    });
  });

  describe('persistence failure', () => {
    it('reports a failed write with a safe typed error and keeps the value dirty', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      const cause = new Error('disk full: secret-title');
      store.failNext('set', cause);

      draft.update({ title: 'secret-title', tags: [] });
      const error = await draft.save().catch((failure: unknown) => failure);

      expect(error).toBeInstanceOf(DraftPersistenceError);
      expect(error).toMatchObject({
        code: 'draft_persistence_failed',
        operation: 'write',
        cause,
      });
      expect((error as Error).message).not.toContain('secret-title');
      expect((error as Error).message).not.toContain('account-a');
      expect(openSnapshot(draft)).toMatchObject({
        persistence: 'failed',
        dirty: true,
        error,
        value: { title: 'secret-title', tags: [] },
      });

      await expect(draft.save()).resolves.toBe('saved');
      expect(openSnapshot(draft)).toMatchObject({ persistence: 'idle', dirty: false, error: null });
    });

    it('normalizes quota failures in the cause', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      store.failNext('set', Object.assign(new Error('quota'), { name: 'QuotaExceededError' }));

      draft.update({ title: 'A', tags: [] });
      const error = await draft.save().catch((failure: unknown) => failure);

      expect((error as DraftPersistenceError).cause).toBeInstanceOf(StorageError);
      expect((error as DraftPersistenceError).cause).toMatchObject({
        code: 'storage_quota_exceeded',
      });
    });

    it('reports a failed read as unavailable and still accepts edits', async () => {
      const store = new ControlledStore();
      store.failNext('get', new Error('blocked storage'));
      const draft = createDraft(store);

      await draft.open(accountA, EMPTY);

      expect(openSnapshot(draft)).toMatchObject({
        recovery: 'unavailable',
        persistence: 'failed',
        value: EMPTY,
      });
      expect(openSnapshot(draft).error).toMatchObject({ operation: 'read' });

      draft.update({ title: 'A', tags: [] });
      await expect(draft.save()).resolves.toBe('saved');
      expect(openSnapshot(draft)).toMatchObject({ persistence: 'idle', error: null });
    });

    it('reports an encoder failure as a failed write', async () => {
      const store = new ControlledStore();
      const draft = createDurableDraft<Scope, FormValue>({
        store,
        key: draftKey,
        codec: {
          ...codecV2,
          encode: () => {
            throw new Error('cannot encode');
          },
        },
      });
      await draft.open(accountA, EMPTY);

      draft.update({ title: 'A', tags: [] });

      await expect(draft.save()).rejects.toMatchObject({ operation: 'write' });
      expect(store.calls.map((call) => call.operation)).toEqual(['get']);
    });
  });

  describe('clearing', () => {
    it('clears after a confirmed submission and keeps the value visible', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Sent', tags: [] });
      await draft.save();

      const { localRevision } = openSnapshot(draft);
      await expect(draft.clear({ reason: 'submitted', localRevision })).resolves.toBe('cleared');

      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Sent', tags: [] },
        dirty: false,
        persistence: 'idle',
        submission: 'confirmed',
      });
      await expect(store.inner.get(draftKey(accountA))).resolves.toBeUndefined();
    });

    it('keeps newer edits instead of clearing after a submission of an older value', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Sent', tags: [] });
      const { localRevision } = openSnapshot(draft);
      draft.update({ title: 'Sent and more', tags: [] });
      await draft.save();

      await expect(draft.clear({ reason: 'submitted', localRevision })).resolves.toBe(
        'newer-edits-kept',
      );

      expect(store.calls.some((call) => call.operation === 'delete')).toBe(false);
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'Sent and more', tags: [] },
      });
    });

    it('keeps an edit made while the delete runs dirty', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Sent', tags: [] });
      await draft.save();
      const gate = store.hold('delete');

      const cleared = draft.clear({
        reason: 'submitted',
        localRevision: openSnapshot(draft).localRevision,
      });
      await settle();
      expect(openSnapshot(draft).persistence).toBe('clearing');
      draft.update({ title: 'Follow-up', tags: [] });
      gate.resolve();

      await expect(cleared).resolves.toBe('cleared');
      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Follow-up', tags: [] },
        dirty: true,
      });
    });

    it('keeps a failed delete visible after a confirmed submission', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Sent', tags: [] });
      await draft.save();
      store.failNext('delete', new Error('locked'));

      const localRevision = openSnapshot(draft).localRevision;
      const error = await draft
        .clear({ reason: 'submitted', localRevision })
        .catch((failure: unknown) => failure);

      expect(error).toBeInstanceOf(DraftPersistenceError);
      expect(error).toMatchObject({ operation: 'delete' });
      expect(openSnapshot(draft)).toMatchObject({
        persistence: 'failed',
        submission: 'confirmed',
        error,
        value: { title: 'Sent', tags: [] },
      });
      await expect(store.inner.get(draftKey(accountA))).resolves.toBeDefined();

      await expect(draft.clear({ reason: 'submitted', localRevision })).resolves.toBe('cleared');
      expect(openSnapshot(draft)).toMatchObject({ persistence: 'idle', error: null });
    });

    it('discards local edits and returns to the initial value', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Unwanted', tags: [] });
      await draft.save();

      await expect(draft.clear({ reason: 'discarded' })).resolves.toBe('cleared');

      expect(openSnapshot(draft)).toMatchObject({ value: EMPTY, dirty: false, recovery: 'none' });
      await expect(store.inner.get(draftKey(accountA))).resolves.toBeUndefined();
    });

    it('keeps the local value when discarding fails', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'Unwanted', tags: [] });
      store.failNext('delete', new Error('locked'));

      await expect(draft.clear({ reason: 'discarded' })).rejects.toBeInstanceOf(
        DraftPersistenceError,
      );
      expect(openSnapshot(draft)).toMatchObject({
        value: { title: 'Unwanted', tags: [] },
        persistence: 'failed',
        submission: 'none',
      });
    });

    it('waits for an in-flight write before deleting', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'A', tags: [] });
      const gate = store.hold('set');

      const saved = draft.save();
      const cleared = draft.clear({ reason: 'discarded' });
      await settle();
      gate.resolve();
      await Promise.all([saved, cleared]);

      expect(store.calls.map((call) => call.operation)).toEqual(['get', 'set', 'delete']);
      await expect(store.inner.get(draftKey(accountA))).resolves.toBeUndefined();
    });
  });

  describe('account and document switching', () => {
    it('ignores a late read from the previous account', async () => {
      const store = new ControlledStore();
      await store.inner.set(draftKey(accountA), { version: 2, value: { title: 'A', tags: [] } });
      await store.inner.set(draftKey(accountB), { version: 2, value: { title: 'B', tags: [] } });
      const gate = store.hold('get');
      const draft = createDraft(store);

      const openA = draft.open(accountA, EMPTY);
      const openB = draft.open(accountB, EMPTY);
      gate.resolve();
      await Promise.all([openA, openB]);

      expect(openSnapshot(draft)).toMatchObject({
        scope: accountB,
        value: { title: 'B', tags: [] },
        recovery: 'restored',
      });
    });

    it('does not publish a late write from the previous document', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'A', tags: [] });
      const gate = store.hold('set');
      const saved = draft.save();
      await settle();

      const opened = draft.open(otherDocument, EMPTY);
      gate.resolve();
      await expect(saved).resolves.toBe('saved');
      await opened;

      expect(openSnapshot(draft)).toMatchObject({
        scope: otherDocument,
        value: EMPTY,
        dirty: false,
        persistence: 'idle',
      });
      await expect(store.inner.get(draftKey(accountA))).resolves.toEqual({
        version: 2,
        value: { title: 'A', tags: [] },
      });
    });

    it('does not publish a late write failure from the previous account', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'A', tags: [] });
      store.failNext('set', new Error('offline'));
      const gate = store.hold('set');
      const saved = draft.save();
      await settle();

      const opened = draft.open(accountB, EMPTY);
      gate.resolve();
      await expect(saved).rejects.toBeInstanceOf(DraftPersistenceError);
      await opened;

      expect(openSnapshot(draft)).toMatchObject({
        scope: accountB,
        persistence: 'idle',
        error: null,
      });
    });

    it('skips queued work of the previous scope that had not started', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      const gate = store.hold('get');
      const openA = draft.open(accountA, EMPTY);
      await settle();
      draft.update({ title: 'A', tags: [] });
      const saved = draft.save();
      const cleared = draft.clear({ reason: 'discarded' });

      const openB = draft.open(accountB, EMPTY);
      gate.resolve();
      await Promise.all([openA, openB]);

      await expect(saved).resolves.toBe('stale');
      await expect(cleared).resolves.toBe('stale');
      expect(store.calls.map((call) => call.operation)).toEqual(['get', 'get']);
    });

    it('sees the settled write of the first A session when switching A to B to A', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'first-a', tags: [] });
      const writeGate = store.hold('set');
      const saved = draft.save();
      await settle();

      const openB = draft.open(accountB, EMPTY);
      const openA = draft.open(accountA, EMPTY);
      writeGate.resolve();
      await Promise.all([saved, openB, openA]);

      expect(openSnapshot(draft)).toMatchObject({
        scope: accountA,
        value: { title: 'first-a', tags: [] },
        recovery: 'restored',
      });
    });

    it('does not let a late delete from the first A session clear the second', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'first-a', tags: [] });
      await draft.save();
      const deleteGate = store.hold('delete');
      const cleared = draft.clear({ reason: 'discarded' });
      await settle();

      const openB = draft.open(accountB, EMPTY);
      const openA = draft.open(accountA, { title: 'initial-a', tags: [] });
      deleteGate.resolve();
      await expect(cleared).resolves.toBe('cleared');
      await Promise.all([openB, openA]);

      expect(openSnapshot(draft)).toMatchObject({
        scope: accountA,
        value: { title: 'initial-a', tags: [] },
        recovery: 'none',
        persistence: 'idle',
      });
    });

    it('never writes one account value under another account key', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'A only', tags: [] });
      await draft.open(accountB, EMPTY);

      await expect(draft.save()).resolves.toBe('clean');
      await expect(store.inner.get(draftKey(accountB))).resolves.toBeUndefined();
    });
  });

  describe('closing', () => {
    it('closes without flushing and ignores later calls', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'unsaved', tags: [] });

      await draft.close();
      draft.update({ title: 'ignored', tags: [] });

      expect(draft.getSnapshot()).toEqual({ open: false });
      await expect(draft.save()).resolves.toBe('stale');
      await expect(draft.clear({ reason: 'discarded' })).resolves.toBe('stale');
      expect(store.calls.map((call) => call.operation)).toEqual(['get']);
    });

    it('waits for the in-flight write before resolving', async () => {
      const store = new ControlledStore();
      const draft = createDraft(store);
      await draft.open(accountA, EMPTY);
      draft.update({ title: 'A', tags: [] });
      const gate = store.hold('set');
      void draft.save();
      await settle();

      let closed = false;
      const closing = draft.close().then(() => {
        closed = true;
      });
      await settle();
      expect(closed).toBe(false);
      gate.resolve();
      await closing;

      expect(closed).toBe(true);
      expect(draft.getSnapshot()).toEqual({ open: false });
    });
  });

  it('notifies subscribers and keeps the snapshot stable between changes', async () => {
    const draft = createDraft(new ControlledStore());
    let notifications = 0;
    const unsubscribe = draft.subscribe(() => {
      notifications += 1;
    });

    await draft.open(accountA, EMPTY);
    const snapshot = draft.getSnapshot();
    const before = notifications;
    expect(draft.getSnapshot()).toBe(snapshot);

    draft.update({ title: 'A', tags: [] });
    expect(notifications).toBe(before + 1);

    unsubscribe();
    draft.update({ title: 'AB', tags: [] });
    expect(notifications).toBe(before + 1);
  });
});

describe('durable draft configuration', () => {
  it('rejects a codec version that is not a positive integer', () => {
    for (const version of [0, -1, 1.5, Number.NaN]) {
      expect(() =>
        createDurableDraft<Scope, FormValue>({
          store: new ControlledStore(),
          key: draftKey,
          codec: { ...codecV2, version },
        }),
      ).toThrow(RangeError);
    }
  });

  it('rejects open and keeps the current scope when the key encoder throws', async () => {
    const draft = createDurableDraft<Scope, FormValue>({
      store: new ControlledStore(),
      key: (scope) => {
        if (scope.account === '') {
          throw new Error('empty account');
        }
        return draftKey(scope);
      },
      codec: codecV2,
    });
    await draft.open(accountA, EMPTY);

    await expect(draft.open({ account: '', document: 'x' }, EMPTY)).rejects.toThrow(
      'empty account',
    );
    expect(openSnapshot(draft).scope).toEqual(accountA);
  });
});

it('exports the draft helper from its own entry point only', async () => {
  const [root, entry] = await Promise.all([
    import('@baukit/data-contracts'),
    import('@baukit/data-contracts/durable-draft'),
  ]);
  expect(entry.createDurableDraft).toBeTypeOf('function');
  expect(entry.DraftPersistenceError).toBeTypeOf('function');
  expect(Object.keys(root)).not.toContain('createDurableDraft');
});

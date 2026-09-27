import { describe, expect, it } from 'vitest';

import {
  type RevisionedWriteQueue,
  type RevisionedWriteQueueOptions,
  type RevisionedWriteRequest,
  type RevisionedWriteResult,
  createRevisionedWriteQueue,
} from './revisioned-writes.js';

interface Deferred<T> {
  readonly promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

type Request<TValue = string> = RevisionedWriteRequest<string, string, TValue, number>;
type Result = RevisionedWriteResult<number, { readonly serverValue: string }>;

interface PendingWrite<TValue> {
  readonly request: Request<TValue>;
  readonly result: Deferred<Result>;
}

class FakeServer<TValue = string> {
  readonly writes: PendingWrite<TValue>[] = [];

  readonly write = (request: Request<TValue>): Promise<Result> => {
    const result = deferred<Result>();
    this.writes.push({ request, result });
    return result.promise;
  };

  at(index: number): PendingWrite<TValue> {
    const write = this.writes[index];
    if (write === undefined) {
      throw new Error(`No write at index ${String(index)}.`);
    }
    return write;
  }

  accept(index: number, revision: number): void {
    this.at(index).result.resolve({ kind: 'accepted', revision });
  }
}

type Options<TValue> = Partial<
  RevisionedWriteQueueOptions<string, string, TValue, number, { readonly serverValue: string }>
>;

function createQueue<TValue = string>(
  server: FakeServer<TValue>,
  options: Options<TValue> = {},
): RevisionedWriteQueue<string, string, TValue, number, { readonly serverValue: string }> {
  return createRevisionedWriteQueue<
    string,
    string,
    TValue,
    number,
    { readonly serverValue: string }
  >({
    initial: { scope: 'account-a', document: 'doc-1', acknowledgedRevision: 1 },
    write: server.write,
    ...options,
  });
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await Promise.resolve();
  }
}

describe('revisioned write queue', () => {
  it('starts idle with the acknowledged revision and no unsent value', () => {
    const queue = createQueue(new FakeServer());

    expect(queue.getSnapshot()).toMatchObject({
      status: 'idle',
      scope: 'account-a',
      document: 'doc-1',
      acknowledgedRevision: 1,
      unsent: null,
      uncertain: null,
      conflict: null,
      error: null,
    });
  });

  it('keeps an enqueued value unsent until the product flushes', async () => {
    const server = new FakeServer();
    const queue = createQueue(server);

    queue.enqueue('draft');
    await settle();

    expect(server.writes).toHaveLength(0);
    expect(queue.getSnapshot()).toMatchObject({
      status: 'dirty',
      acknowledgedRevision: 1,
      unsent: { value: 'draft' },
    });
  });

  describe('edits during save', () => {
    it('keeps a newer edit dirty while the older write is in flight', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);
      const statuses: string[] = [];
      queue.subscribe(() => statuses.push(queue.getSnapshot().status));

      queue.enqueue('first');
      const flushed = queue.flush();
      queue.enqueue('second');

      expect(queue.getSnapshot()).toMatchObject({
        status: 'writing',
        acknowledgedRevision: 1,
        unsent: { value: 'second' },
      });

      server.accept(0, 2);
      await settle();

      expect(server.writes).toHaveLength(2);
      expect(server.at(1).request).toMatchObject({ value: 'second', expectedRevision: 2 });
      expect(queue.getSnapshot()).toMatchObject({
        status: 'writing',
        acknowledgedRevision: 2,
        unsent: null,
      });

      server.accept(1, 3);
      const snapshot = await flushed;

      expect(snapshot).toMatchObject({ status: 'idle', acknowledgedRevision: 3, unsent: null });
      expect(statuses.slice(0, -1)).not.toContain('idle');
    });

    it('never lets an older acknowledgement mark a newer value saved', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('first');
      void queue.flush();
      queue.enqueue('second');
      server.at(0).result.resolve({ kind: 'rejected', error: new Error('offline') });
      await settle();

      expect(queue.getSnapshot()).toMatchObject({
        status: 'failed',
        acknowledgedRevision: 1,
        unsent: { value: 'second' },
      });
    });
  });

  describe('several queued writes', () => {
    it('coalesces three queued values to the newest by default', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      const flushed = queue.flush();
      queue.enqueue('b');
      queue.enqueue('c');
      queue.enqueue('d');
      server.accept(0, 2);
      await settle();
      server.accept(1, 3);
      await flushed;

      expect(server.writes.map((write) => write.request.value)).toEqual(['a', 'd']);
      expect(server.writes.map((write) => write.request.expectedRevision)).toEqual([1, 2]);
      expect(queue.getSnapshot()).toMatchObject({ status: 'idle', acknowledgedRevision: 3 });
    });

    it('lets a custom coalescer keep every operation', async () => {
      const server = new FakeServer<readonly string[]>();
      const queue = createQueue(server, {
        coalesce: (older, newer) => [...older, ...newer],
      });

      queue.enqueue(['a']);
      const flushed = queue.flush();
      queue.enqueue(['b']);
      queue.enqueue(['c']);
      queue.enqueue(['d']);
      server.accept(0, 2);
      await settle();
      server.accept(1, 3);
      await flushed;

      expect(server.writes.map((write) => write.request.value)).toEqual([['a'], ['b', 'c', 'd']]);
    });

    it('sends values that arrive between writes in order with successive revisions', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      const flushed = queue.flush();
      queue.enqueue('b');
      server.accept(0, 2);
      await settle();
      queue.enqueue('c');
      server.accept(1, 3);
      await settle();
      server.accept(2, 4);
      await flushed;

      expect(
        server.writes.map((write) => [write.request.value, write.request.expectedRevision]),
      ).toEqual([
        ['a', 1],
        ['b', 2],
        ['c', 3],
      ]);
      expect(queue.getSnapshot().acknowledgedRevision).toBe(4);
    });
  });

  describe('concurrent flushes', () => {
    it('shares one drain between concurrent callers', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('value');
      const first = queue.flush();
      const second = queue.flush();
      queue.enqueue('later');
      const third = queue.flush();

      expect(second).toBe(first);
      expect(third).toBe(first);
      expect(server.writes).toHaveLength(1);

      server.accept(0, 2);
      await settle();
      server.accept(1, 3);
      const snapshots = await Promise.all([first, second, third]);

      expect(server.writes).toHaveLength(2);
      expect(snapshots.every((snapshot) => snapshot.status === 'idle')).toBe(true);
    });

    it('starts a new drain for a value enqueued after the previous drain finished', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      const first = queue.flush();
      server.accept(0, 2);
      await first;
      queue.enqueue('b');
      const second = queue.flush();

      expect(second).not.toBe(first);
      expect(server.at(1).request).toMatchObject({ value: 'b', expectedRevision: 2 });
    });

    it('resolves immediately when nothing is unsent', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      await expect(queue.flush()).resolves.toMatchObject({ status: 'idle' });
      expect(server.writes).toHaveLength(0);
    });
  });

  describe('retry after a definite failure', () => {
    it('keeps the acknowledged revision and resends the merged unsent value', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);
      const failure = new Error('validation');

      queue.enqueue('a');
      void queue.flush();
      server.at(0).result.resolve({ kind: 'rejected', error: failure });
      await settle();

      expect(queue.getSnapshot()).toMatchObject({
        status: 'failed',
        acknowledgedRevision: 1,
        unsent: { value: 'a' },
        error: failure,
      });

      queue.enqueue('b');
      const retried = queue.retry();

      expect(server.at(1).request).toMatchObject({
        value: 'b',
        expectedRevision: 1,
        afterUnknownOutcome: false,
      });
      server.accept(1, 2);

      await expect(retried).resolves.toMatchObject({
        status: 'idle',
        acknowledgedRevision: 2,
        error: null,
      });
    });

    it('lets a later flush resume after a definite failure', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();
      server.at(0).result.resolve({ kind: 'rejected', error: new Error('offline') });
      await settle();
      void queue.flush();

      expect(server.at(1).request).toMatchObject({ value: 'a', expectedRevision: 1 });
    });

    it('merges the failed value with newer edits through the coalescer', async () => {
      const server = new FakeServer<readonly string[]>();
      const queue = createQueue(server, { coalesce: (older, newer) => [...older, ...newer] });

      queue.enqueue(['a']);
      void queue.flush();
      queue.enqueue(['b']);
      server.at(0).result.resolve({ kind: 'rejected', error: new Error('offline') });
      await settle();

      expect(queue.getSnapshot().unsent).toEqual({ value: ['a', 'b'] });
    });
  });

  describe('ambiguous completion', () => {
    it('pauses on an unknown outcome and never folds it into newer edits', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);
      const failure = new Error('connection reset');

      queue.enqueue('a');
      void queue.flush();
      queue.enqueue('b');
      server.at(0).result.reject(failure);
      await settle();

      expect(queue.getSnapshot()).toMatchObject({
        status: 'unknown-outcome',
        acknowledgedRevision: 1,
        uncertain: { value: 'a', expectedRevision: 1 },
        unsent: { value: 'b' },
        error: failure,
      });

      await queue.flush();
      expect(server.writes).toHaveLength(1);
    });

    it('replays the identical write with a flag when the product retries', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();
      queue.enqueue('b');
      server.at(0).result.reject(new Error('timeout'));
      await settle();

      const retried = queue.retry();
      expect(server.at(1).request).toMatchObject({
        value: 'a',
        expectedRevision: 1,
        afterUnknownOutcome: true,
      });
      expect(queue.getSnapshot()).toMatchObject({ status: 'writing', uncertain: null });

      server.accept(1, 2);
      await settle();
      expect(server.at(2).request).toMatchObject({
        value: 'b',
        expectedRevision: 2,
        afterUnknownOutcome: false,
      });
      server.accept(2, 3);

      await expect(retried).resolves.toMatchObject({ status: 'idle', acknowledgedRevision: 3 });
    });

    it('treats a synchronous throw from the write callback as an unknown outcome', async () => {
      const queue = createRevisionedWriteQueue<string, string, string, number, never>({
        initial: { scope: 'account-a', document: 'doc-1', acknowledgedRevision: 1 },
        write: () => {
          throw new Error('boom');
        },
      });

      queue.enqueue('a');
      await expect(queue.flush()).resolves.toMatchObject({
        status: 'unknown-outcome',
        uncertain: { value: 'a', expectedRevision: 1 },
      });
    });

    it('lets the product reseed after checking the server instead of replaying', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();
      server.at(0).result.reject(new Error('timeout'));
      await settle();
      queue.reset({ scope: 'account-a', document: 'doc-1', acknowledgedRevision: 2 });

      expect(queue.getSnapshot()).toMatchObject({
        status: 'idle',
        acknowledgedRevision: 2,
        uncertain: null,
        error: null,
      });
    });
  });

  describe('conflict', () => {
    it('stops queued writes and keeps the conflicting value unsent', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();
      queue.enqueue('b');
      server.at(0).result.resolve({
        kind: 'conflict',
        currentRevision: 5,
        conflict: { serverValue: 'theirs' },
      });
      await settle();

      expect(server.writes).toHaveLength(1);
      expect(queue.getSnapshot()).toMatchObject({
        status: 'conflict',
        acknowledgedRevision: 1,
        unsent: { value: 'b' },
        conflict: { currentRevision: 5, conflict: { serverValue: 'theirs' } },
      });

      queue.enqueue('c');
      await queue.flush();
      await queue.retry();
      expect(server.writes).toHaveLength(1);
      expect(queue.getSnapshot()).toMatchObject({ status: 'conflict', unsent: { value: 'c' } });
    });

    it('resumes only after the product resets to the current revision', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('mine');
      void queue.flush();
      server.at(0).result.resolve({
        kind: 'conflict',
        currentRevision: 5,
        conflict: { serverValue: 'theirs' },
      });
      await settle();

      queue.reset({ scope: 'account-a', document: 'doc-1', acknowledgedRevision: 5 });
      queue.enqueue('mine');
      const flushed = queue.flush();
      expect(server.at(1).request).toMatchObject({ value: 'mine', expectedRevision: 5 });
      server.accept(1, 6);

      await expect(flushed).resolves.toMatchObject({
        status: 'idle',
        acknowledgedRevision: 6,
        conflict: null,
      });
    });
  });

  describe('cancellation', () => {
    it('aborts the active write, drops unsent values, and ignores the late result', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      const flushed = queue.flush();
      queue.enqueue('b');
      queue.cancel();

      expect(server.at(0).request.signal.aborted).toBe(true);
      expect(queue.getSnapshot()).toMatchObject({
        status: 'cancelled',
        acknowledgedRevision: 1,
        unsent: null,
      });

      server.accept(0, 2);
      await expect(flushed).resolves.toMatchObject({ status: 'cancelled' });
      await settle();
      expect(server.writes).toHaveLength(1);
      expect(queue.getSnapshot().acknowledgedRevision).toBe(1);
    });

    it('ignores enqueue, flush, and retry after cancellation', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.cancel();
      queue.enqueue('a');
      await queue.flush();
      await queue.retry();

      expect(server.writes).toHaveLength(0);
      expect(queue.getSnapshot()).toMatchObject({ status: 'cancelled', unsent: null });
    });

    it('cancels when the injected signal aborts', async () => {
      const server = new FakeServer();
      const host = new AbortController();
      const queue = createQueue(server, { signal: host.signal });

      queue.enqueue('a');
      void queue.flush();
      host.abort();
      server.accept(0, 2);
      await settle();

      expect(server.at(0).request.signal.aborted).toBe(true);
      expect(queue.getSnapshot()).toMatchObject({ status: 'cancelled', acknowledgedRevision: 1 });
    });

    it('starts cancelled when the injected signal already aborted', () => {
      const host = new AbortController();
      host.abort();
      const queue = createQueue(new FakeServer(), { signal: host.signal });

      queue.reset({ scope: 'account-b', document: 'doc-2', acknowledgedRevision: 1 });
      expect(queue.getSnapshot().status).toBe('cancelled');
    });

    it('can be reused after a product reset', () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.cancel();
      queue.reset({ scope: 'account-a', document: 'doc-1', acknowledgedRevision: 7 });
      queue.enqueue('a');
      void queue.flush();

      expect(server.at(0).request).toMatchObject({ value: 'a', expectedRevision: 7 });
    });
  });

  describe('scope changes', () => {
    it('fences a late completion after an account switch', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a-value');
      const oldFlush = queue.flush();
      queue.enqueue('a-later');
      queue.reset({ scope: 'account-b', document: 'doc-9', acknowledgedRevision: 40 });

      expect(server.at(0).request.signal.aborted).toBe(true);
      expect(queue.getSnapshot()).toMatchObject({
        status: 'idle',
        scope: 'account-b',
        document: 'doc-9',
        acknowledgedRevision: 40,
        unsent: null,
      });

      server.accept(0, 2);
      await oldFlush;
      await settle();

      expect(server.writes).toHaveLength(1);
      expect(queue.getSnapshot()).toMatchObject({ scope: 'account-b', acknowledgedRevision: 40 });
    });

    it('fences late failures and conflicts after a document switch', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();
      queue.reset({ scope: 'account-a', document: 'doc-2', acknowledgedRevision: 10 });
      queue.enqueue('b');
      void queue.flush();

      server.at(0).result.resolve({
        kind: 'conflict',
        currentRevision: 3,
        conflict: { serverValue: 'x' },
      });
      await settle();

      expect(queue.getSnapshot()).toMatchObject({
        status: 'writing',
        document: 'doc-2',
        conflict: null,
      });
      expect(server.at(1).request).toMatchObject({
        document: 'doc-2',
        value: 'b',
        expectedRevision: 10,
      });

      server.accept(1, 11);
      await settle();
      expect(queue.getSnapshot()).toMatchObject({ status: 'idle', acknowledgedRevision: 11 });
    });

    it('ignores a late A write after switching A to B and back to A', async () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('first-a');
      void queue.flush();
      queue.reset({ scope: 'account-b', document: 'doc-1', acknowledgedRevision: 1 });
      queue.reset({ scope: 'account-a', document: 'doc-1', acknowledgedRevision: 3 });
      queue.enqueue('second-a');
      const flushed = queue.flush();

      server.at(0).result.reject(new Error('aborted'));
      await settle();
      expect(queue.getSnapshot()).toMatchObject({ status: 'writing', uncertain: null });

      server.accept(1, 4);
      await expect(flushed).resolves.toMatchObject({
        status: 'idle',
        scope: 'account-a',
        acknowledgedRevision: 4,
      });
    });

    it('passes the scope and document of the generation that started the write', () => {
      const server = new FakeServer();
      const queue = createQueue(server);

      queue.enqueue('a');
      void queue.flush();

      expect(server.at(0).request).toMatchObject({ scope: 'account-a', document: 'doc-1' });
    });
  });

  describe('observable snapshot', () => {
    it('notifies subscribers and keeps the snapshot stable between changes', () => {
      const queue = createQueue(new FakeServer());
      let notifications = 0;
      const unsubscribe = queue.subscribe(() => {
        notifications += 1;
      });

      const before = queue.getSnapshot();
      expect(queue.getSnapshot()).toBe(before);
      queue.enqueue('a');
      expect(notifications).toBe(1);
      expect(queue.getSnapshot()).not.toBe(before);

      unsubscribe();
      queue.enqueue('b');
      expect(notifications).toBe(1);
    });
  });
});

it('exports the queue from its own entry point only', async () => {
  const [root, entry] = await Promise.all([
    import('@baukit/data-contracts'),
    import('@baukit/data-contracts/revisioned-writes'),
  ]);
  expect(entry.createRevisionedWriteQueue).toBeTypeOf('function');
  expect(Object.keys(root)).not.toContain('createRevisionedWriteQueue');
});

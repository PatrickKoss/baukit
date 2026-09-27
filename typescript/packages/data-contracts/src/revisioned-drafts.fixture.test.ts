import { describe, expect, it } from 'vitest';

import { InMemoryKeyValueStore, type JsonValue, type KeyValueStore } from '@baukit/data-contracts';
import {
  type DraftCodec,
  type DurableDraft,
  createDurableDraft,
} from '@baukit/data-contracts/durable-draft';
import {
  type RevisionedWriteQueue,
  type RevisionedWriteRequest,
  type RevisionedWriteResult,
  createRevisionedWriteQueue,
} from '@baukit/data-contracts/revisioned-writes';

/**
 * A plain-object editor that composes both helpers the way a product hook would: the draft keeps a
 * crash-safe local copy, the queue sends revision-checked writes, and the draft is cleared only
 * after the server acknowledged the exact local revision.
 */

interface Scope {
  readonly account: string;
  readonly note: string;
}

interface NoteDraft {
  readonly body: string;
  /** The server revision the local edits started from, like an ETag kept with the draft. */
  readonly baseRevision: number;
}

interface ServerConflict {
  readonly body: string;
}

type NoteQueue = RevisionedWriteQueue<string, string, string, number, ServerConflict>;

class FakeNoteServer {
  readonly notes = new Map<string, { body: string; revision: number }>();
  loseNextResponse = false;

  seed(scope: Scope, body: string, revision: number): void {
    this.notes.set(this.#key(scope.account, scope.note), { body, revision });
  }

  read(scope: Scope): { readonly body: string; readonly revision: number } {
    return this.notes.get(this.#key(scope.account, scope.note)) ?? { body: '', revision: 0 };
  }

  readonly write = (
    request: RevisionedWriteRequest<string, string, string, number>,
  ): Promise<RevisionedWriteResult<number, ServerConflict>> => {
    const key = this.#key(request.scope, request.document);
    const current = this.notes.get(key) ?? { body: '', revision: 0 };
    if (current.revision !== request.expectedRevision) {
      return Promise.resolve({
        kind: 'conflict',
        currentRevision: current.revision,
        conflict: { body: current.body },
      });
    }
    const next = { body: request.value, revision: current.revision + 1 };
    this.notes.set(key, next);
    if (this.loseNextResponse) {
      this.loseNextResponse = false;
      return Promise.reject(new Error('response lost'));
    }
    return Promise.resolve({ kind: 'accepted', revision: next.revision });
  };

  #key(account: string, note: string): string {
    return `${account}/${note}`;
  }
}

const noteCodec: DraftCodec<NoteDraft> = {
  version: 1,
  encode: (value) => ({ body: value.body, baseRevision: value.baseRevision }),
  decode(value: JsonValue) {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
      return { kind: 'corrupt' };
    }
    const body = value['body'];
    const baseRevision = value['baseRevision'];
    if (typeof body !== 'string' || typeof baseRevision !== 'number') {
      return { kind: 'corrupt' };
    }
    return { kind: 'decoded', value: { body, baseRevision } };
  },
};

class NoteEditor {
  readonly draft: DurableDraft<Scope, NoteDraft>;
  readonly queue: NoteQueue;

  constructor(
    store: KeyValueStore,
    private readonly server: FakeNoteServer,
    private scope: Scope,
  ) {
    this.draft = createDurableDraft({
      store,
      key: (next: Scope) => `note-draft:${next.account}:${next.note}`,
      codec: noteCodec,
    });
    const loaded = server.read(scope);
    this.queue = createRevisionedWriteQueue({
      initial: {
        scope: scope.account,
        document: scope.note,
        acknowledgedRevision: loaded.revision,
      },
      write: server.write,
    });
  }

  get body(): string {
    const snapshot = this.draft.getSnapshot();
    return snapshot.open ? snapshot.value.body : '';
  }

  async start(): Promise<void> {
    const loaded = this.server.read(this.scope);
    this.queue.reset({
      scope: this.scope.account,
      document: this.scope.note,
      acknowledgedRevision: loaded.revision,
    });
    await this.draft.open(this.scope, { body: loaded.body, baseRevision: loaded.revision });
    const snapshot = this.draft.getSnapshot();
    if (!snapshot.open || snapshot.recovery !== 'restored') {
      return;
    }
    this.queue.reset({
      scope: this.scope.account,
      document: this.scope.note,
      acknowledgedRevision: snapshot.value.baseRevision,
    });
    this.queue.enqueue(snapshot.value.body);
  }

  edit(body: string): void {
    const baseRevision = this.queue.getSnapshot().acknowledgedRevision;
    this.draft.update({ body, baseRevision });
    this.queue.enqueue(body);
  }

  async tick(): Promise<void> {
    await this.draft.save();
    const draft = this.draft.getSnapshot();
    if (!draft.open) {
      return;
    }
    const written = await this.queue.flush();
    if (written.status === 'idle' && written.scope === this.scope.account) {
      await this.draft.clear({ reason: 'submitted', localRevision: draft.localRevision });
    }
  }

  async switchTo(scope: Scope): Promise<void> {
    this.scope = scope;
    await this.start();
  }
}

describe('revisioned drafts fixture', () => {
  it('saves locally, writes to the server, then clears the draft', async () => {
    const store = new InMemoryKeyValueStore();
    const server = new FakeNoteServer();
    server.seed({ account: 'a', note: 'n1' }, 'hello', 3);
    const editor = new NoteEditor(store, server, { account: 'a', note: 'n1' });
    await editor.start();

    editor.edit('hello world');
    await editor.tick();

    expect(server.read({ account: 'a', note: 'n1' })).toEqual({ body: 'hello world', revision: 4 });
    expect(editor.queue.getSnapshot()).toMatchObject({ status: 'idle', acknowledgedRevision: 4 });
    expect(editor.draft.getSnapshot()).toMatchObject({ submission: 'confirmed', dirty: false });
    await expect(store.get('note-draft:a:n1')).resolves.toBeUndefined();
  });

  it('recovers a draft after a lost response and reports the conflict it caused', async () => {
    const store = new InMemoryKeyValueStore();
    const server = new FakeNoteServer();
    server.seed({ account: 'a', note: 'n1' }, 'v1', 1);
    const first = new NoteEditor(store, server, { account: 'a', note: 'n1' });
    await first.start();
    first.edit('v2 local');
    server.loseNextResponse = true;
    await first.tick();

    expect(first.queue.getSnapshot()).toMatchObject({
      status: 'unknown-outcome',
      uncertain: { value: 'v2 local', expectedRevision: 1 },
    });
    await expect(store.get('note-draft:a:n1')).resolves.toEqual({
      version: 1,
      value: { body: 'v2 local', baseRevision: 1 },
    });

    const reopened = new NoteEditor(store, server, { account: 'a', note: 'n1' });
    await reopened.start();
    expect(reopened.body).toBe('v2 local');
    await reopened.tick();

    expect(reopened.queue.getSnapshot()).toMatchObject({
      status: 'conflict',
      acknowledgedRevision: 1,
      conflict: { currentRevision: 2, conflict: { body: 'v2 local' } },
    });
    expect(reopened.draft.getSnapshot()).toMatchObject({ dirty: false, submission: 'none' });
    await expect(store.get('note-draft:a:n1')).resolves.toBeDefined();
  });

  it('keeps a late acknowledgement for one account out of the next account', async () => {
    const store = new InMemoryKeyValueStore();
    const server = new FakeNoteServer();
    server.seed({ account: 'a', note: 'n1' }, 'a-body', 1);
    server.seed({ account: 'b', note: 'n1' }, 'b-body', 9);
    const editor = new NoteEditor(store, server, { account: 'a', note: 'n1' });
    await editor.start();

    editor.edit('a-edit');
    await editor.draft.save();
    const lateFlush = editor.queue.flush();
    await editor.switchTo({ account: 'b', note: 'n1' });
    editor.edit('b-edit');
    await editor.draft.save();
    await lateFlush;

    expect(editor.queue.getSnapshot()).toMatchObject({
      scope: 'b',
      acknowledgedRevision: 9,
      unsent: { value: 'b-edit' },
    });
    await expect(store.get('note-draft:b:n1')).resolves.toEqual({
      version: 1,
      value: { body: 'b-edit', baseRevision: 9 },
    });
    await expect(store.get('note-draft:a:n1')).resolves.toEqual({
      version: 1,
      value: { body: 'a-edit', baseRevision: 1 },
    });
  });
});

import { type JsonValue, type KeyValueStore, normalizeStorageError } from './contracts.js';

/** The result of decoding a stored draft value. Return `corrupt` for any value that fails validation. */
export type DraftDecodeResult<TValue> =
  | { readonly kind: 'decoded'; readonly value: TValue }
  | { readonly kind: 'corrupt' }
  | { readonly kind: 'unsupported-version' };

/**
 * Converts a form value to and from stored JSON. The helper stores `{ version, value }` and never
 * calls `decode` for a version above `version`. `decode` receives unchecked JSON and must validate
 * it; an older `version` must be upgraded to the current value shape.
 */
export interface DraftCodec<TValue> {
  /** The positive integer written with every save. */
  readonly version: number;
  encode(value: TValue): JsonValue;
  decode(value: JsonValue, version: number): DraftDecodeResult<TValue>;
}

export type DraftPersistenceOperation = 'read' | 'write' | 'delete';

/** A storage failure. The message names only the operation, never keys or draft content. */
export class DraftPersistenceError extends Error {
  public override readonly name = 'DraftPersistenceError';
  public readonly code = 'draft_persistence_failed';

  public constructor(
    public readonly operation: DraftPersistenceOperation,
    options?: ErrorOptions,
  ) {
    super(`Draft ${operation} failed.`, options);
  }
}

/** What `open` found in storage. */
export type DraftRecovery = 'none' | 'restored' | 'corrupt' | 'unsupported-version' | 'unavailable';

export type DraftPersistence = 'loading' | 'idle' | 'saving' | 'clearing' | 'failed';

export type DraftSubmission = 'none' | 'confirmed';

export interface ClosedDurableDraftSnapshot {
  readonly open: false;
}

export interface OpenDurableDraftSnapshot<TScope, TValue> {
  readonly open: true;
  readonly scope: TScope;
  readonly value: TValue;
  /** Increments on every local change. Pass it to `clear` after a submission. */
  readonly localRevision: number;
  /** True when `value` differs from what storage holds for this scope. */
  readonly dirty: boolean;
  readonly recovery: DraftRecovery;
  readonly persistence: DraftPersistence;
  readonly error: DraftPersistenceError | null;
  /** `confirmed` once the product reported a successful server submission through `clear`. */
  readonly submission: DraftSubmission;
}

export type DurableDraftSnapshot<TScope, TValue> =
  ClosedDurableDraftSnapshot | OpenDurableDraftSnapshot<TScope, TValue>;

export type DraftClearRequest =
  | { readonly reason: 'discarded' }
  | { readonly reason: 'submitted'; readonly localRevision: number };

/** `stale` means the scope changed or closed before the operation started, so storage was not touched. */
export type DraftSaveOutcome = 'saved' | 'clean' | 'blocked' | 'stale';
export type DraftClearOutcome = 'cleared' | 'newer-edits-kept' | 'stale';

export interface DurableDraftOptions<TScope, TValue> {
  readonly store: KeyValueStore;
  /** Encodes the scope to a storage key. Include the identity and document. */
  readonly key: (scope: TScope) => string;
  readonly codec: DraftCodec<TValue>;
}

export interface DurableDraft<TScope, TValue> {
  getSnapshot(): DurableDraftSnapshot<TScope, TValue>;
  subscribe(listener: () => void): () => void;
  /** Switches to a scope and reads its stored draft. Late work of the previous scope is ignored. */
  open(scope: TScope, initial: TValue): Promise<void>;
  /** Replaces the value in memory. Debounce and call `save` to persist it. */
  update(value: TValue): void;
  /**
   * Writes the current value. Resolves `blocked` without writing while recovery is `corrupt` or
   * `unsupported-version`. Rejects with `DraftPersistenceError` when storage fails.
   */
  save(): Promise<DraftSaveOutcome>;
  /**
   * Deletes the stored draft. `submitted` keeps the value visible and keeps newer edits made after
   * `localRevision`; `discarded` returns to the initial value. Rejects with a `delete`
   * `DraftPersistenceError` that stays in the snapshot.
   */
  clear(request: DraftClearRequest): Promise<DraftClearOutcome>;
  /** Closes the scope without saving and resolves after in-flight storage work settles. */
  close(): Promise<void>;
}

interface Session<TScope, TValue> {
  readonly scope: TScope;
  readonly key: string;
  readonly initial: TValue;
}

type LoadResult<TValue> =
  | { readonly recovery: 'none' }
  | { readonly recovery: 'restored'; readonly value: TValue; readonly upgraded: boolean }
  | { readonly recovery: 'corrupt' | 'unsupported-version' };

function isEnvelope(stored: JsonValue): stored is { version: number; value: JsonValue } {
  if (typeof stored !== 'object' || stored === null || Array.isArray(stored)) {
    return false;
  }
  const version = stored['version'];
  return (
    typeof version === 'number' && Number.isSafeInteger(version) && version > 0 && 'value' in stored
  );
}

function decodeStored<TValue>(
  stored: JsonValue | undefined,
  codec: DraftCodec<TValue>,
): LoadResult<TValue> {
  if (stored === undefined) {
    return { recovery: 'none' };
  }
  if (!isEnvelope(stored)) {
    return { recovery: 'corrupt' };
  }
  if (stored.version > codec.version) {
    return { recovery: 'unsupported-version' };
  }
  let result: DraftDecodeResult<TValue>;
  try {
    result = codec.decode(stored.value, stored.version);
  } catch {
    return { recovery: 'corrupt' };
  }
  if (result.kind !== 'decoded') {
    return { recovery: result.kind };
  }
  return {
    recovery: 'restored',
    value: result.value,
    upgraded: stored.version < codec.version,
  };
}

function isBlocked(recovery: DraftRecovery): boolean {
  return recovery === 'corrupt' || recovery === 'unsupported-version';
}

function persistenceError(
  operation: DraftPersistenceOperation,
  cause: unknown,
): DraftPersistenceError {
  return new DraftPersistenceError(operation, { cause: normalizeStorageError(cause) });
}

const CLOSED: ClosedDurableDraftSnapshot = { open: false };

class DefaultDurableDraft<TScope, TValue> implements DurableDraft<TScope, TValue> {
  readonly #store: KeyValueStore;
  readonly #key: (scope: TScope) => string;
  readonly #codec: DraftCodec<TValue>;
  readonly #listeners = new Set<() => void>();
  #session: Session<TScope, TValue> | null = null;
  #persistedRevision: number | null = 0;
  #tail: Promise<unknown> = Promise.resolve();
  #snapshot: DurableDraftSnapshot<TScope, TValue> = CLOSED;

  constructor(options: DurableDraftOptions<TScope, TValue>) {
    const { version } = options.codec;
    if (!Number.isSafeInteger(version) || version < 1) {
      throw new RangeError('Draft codec version must be a positive integer.');
    }
    this.#store = options.store;
    this.#key = options.key;
    this.#codec = options.codec;
  }

  getSnapshot(): DurableDraftSnapshot<TScope, TValue> {
    return this.#snapshot;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  open(scope: TScope, initial: TValue): Promise<void> {
    let key: string;
    try {
      key = this.#key(scope);
    } catch (error) {
      return Promise.reject(error instanceof Error ? error : new Error(String(error)));
    }
    const session: Session<TScope, TValue> = { scope, key, initial };
    this.#session = session;
    this.#persistedRevision = 0;
    this.#publish({
      open: true,
      scope,
      value: initial,
      localRevision: 0,
      dirty: false,
      recovery: 'none',
      persistence: 'loading',
      error: null,
      submission: 'none',
    });
    return this.#enqueue(() => this.#load(session));
  }

  update(value: TValue): void {
    const current = this.#current();
    if (current === null) {
      return;
    }
    this.#publishOpen({
      value,
      localRevision: current.localRevision + 1,
      submission: 'none',
    });
  }

  save(): Promise<DraftSaveOutcome> {
    const session = this.#session;
    if (session === null) {
      return Promise.resolve('stale');
    }
    return this.#enqueue(() => this.#write(session));
  }

  clear(request: DraftClearRequest): Promise<DraftClearOutcome> {
    const session = this.#session;
    if (session === null) {
      return Promise.resolve('stale');
    }
    return this.#enqueue(() => this.#delete(session, request));
  }

  async close(): Promise<void> {
    this.#session = null;
    this.#publish(CLOSED);
    await this.#tail;
  }

  async #load(session: Session<TScope, TValue>): Promise<void> {
    if (session !== this.#session) {
      return;
    }
    let stored: JsonValue | undefined;
    try {
      stored = await this.#store.get(session.key);
    } catch (cause) {
      this.#publishFor(session, {
        recovery: 'unavailable',
        persistence: 'failed',
        error: persistenceError('read', cause),
      });
      return;
    }
    const current = this.#currentFor(session);
    if (current === null) {
      return;
    }
    const loaded = decodeStored(stored, this.#codec);
    if (loaded.recovery === 'restored' && current.localRevision === 0) {
      this.#persistedRevision = loaded.upgraded ? null : 0;
      this.#publishOpen({ value: loaded.value, recovery: 'restored', persistence: 'idle' });
      return;
    }
    this.#publishOpen({
      recovery: loaded.recovery === 'restored' ? 'none' : loaded.recovery,
      persistence: 'idle',
    });
  }

  async #write(session: Session<TScope, TValue>): Promise<DraftSaveOutcome> {
    const current = this.#currentFor(session);
    if (current === null) {
      return 'stale';
    }
    if (!current.dirty) {
      return 'clean';
    }
    if (isBlocked(current.recovery)) {
      return 'blocked';
    }
    const { localRevision, value } = current;
    this.#publishOpen({ persistence: 'saving', error: null });
    try {
      const envelope = { version: this.#codec.version, value: this.#codec.encode(value) };
      await this.#store.set(session.key, envelope);
    } catch (cause) {
      const error = persistenceError('write', cause);
      this.#publishFor(session, { persistence: 'failed', error });
      throw error;
    }
    if (this.#currentFor(session) !== null) {
      this.#persistedRevision = localRevision;
      this.#publishOpen({ persistence: 'idle', error: null });
    }
    return 'saved';
  }

  async #delete(
    session: Session<TScope, TValue>,
    request: DraftClearRequest,
  ): Promise<DraftClearOutcome> {
    const current = this.#currentFor(session);
    if (current === null) {
      return 'stale';
    }
    const submitted = request.reason === 'submitted';
    if (submitted && request.localRevision !== current.localRevision) {
      this.#publishOpen({ submission: 'confirmed' });
      return 'newer-edits-kept';
    }
    const { localRevision } = current;
    const submission: DraftSubmission = submitted ? 'confirmed' : current.submission;
    this.#publishOpen({ persistence: 'clearing', error: null, submission });
    try {
      await this.#store.delete(session.key);
    } catch (cause) {
      const error = persistenceError('delete', cause);
      this.#publishFor(session, { persistence: 'failed', error });
      throw error;
    }
    if (this.#currentFor(session) !== null) {
      this.#applyCleared(session, localRevision, submitted);
    }
    return 'cleared';
  }

  #applyCleared(session: Session<TScope, TValue>, localRevision: number, submitted: boolean): void {
    const base = { recovery: 'none', persistence: 'idle', error: null } as const;
    if (submitted) {
      this.#persistedRevision = localRevision;
      this.#publishOpen(base);
      return;
    }
    const current = this.#current();
    const nextRevision = (current?.localRevision ?? localRevision) + 1;
    this.#persistedRevision = nextRevision;
    this.#publishOpen({
      ...base,
      value: session.initial,
      localRevision: nextRevision,
      submission: 'none',
    });
  }

  #enqueue<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const run = this.#tail.then(operation);
    this.#tail = run.catch(() => undefined);
    return run;
  }

  #current(): OpenDurableDraftSnapshot<TScope, TValue> | null {
    return this.#snapshot.open ? this.#snapshot : null;
  }

  #currentFor(session: Session<TScope, TValue>): OpenDurableDraftSnapshot<TScope, TValue> | null {
    return session === this.#session ? this.#current() : null;
  }

  #publishFor(
    session: Session<TScope, TValue>,
    changes: Partial<OpenDurableDraftSnapshot<TScope, TValue>>,
  ): void {
    if (this.#currentFor(session) !== null) {
      this.#publishOpen(changes);
    }
  }

  #publishOpen(changes: Partial<OpenDurableDraftSnapshot<TScope, TValue>>): void {
    const current = this.#current();
    if (current === null) {
      return;
    }
    const next = { ...current, ...changes };
    this.#publish({ ...next, dirty: next.localRevision !== this.#persistedRevision });
  }

  #publish(snapshot: DurableDraftSnapshot<TScope, TValue>): void {
    this.#snapshot = snapshot;
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

/** Creates a framework-free durable draft over a `KeyValueStore`. */
export function createDurableDraft<TScope, TValue>(
  options: DurableDraftOptions<TScope, TValue>,
): DurableDraft<TScope, TValue> {
  return new DefaultDurableDraft(options);
}

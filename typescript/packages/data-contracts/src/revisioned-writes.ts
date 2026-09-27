/// <reference lib="dom" />

/**
 * The document a queue writes to. `acknowledgedRevision` is the last revision the server
 * confirmed for this document, never a revision derived from a local value that was not sent.
 */
export interface RevisionedWriteScope<TScope, TDocument, TRevision> {
  readonly scope: TScope;
  readonly document: TDocument;
  readonly acknowledgedRevision: TRevision;
}

/**
 * The product-mapped outcome of one write. Return `rejected` only when the server certainly did
 * not apply the write. Throw, or let the promise reject, when the outcome is unknown.
 */
export type RevisionedWriteResult<TRevision, TConflict> =
  | { readonly kind: 'accepted'; readonly revision: TRevision }
  | {
      readonly kind: 'conflict';
      readonly currentRevision: TRevision;
      readonly conflict: TConflict;
    }
  | { readonly kind: 'rejected'; readonly error: unknown };

export interface RevisionedWriteRequest<TScope, TDocument, TValue, TRevision> {
  readonly scope: TScope;
  readonly document: TDocument;
  readonly value: TValue;
  readonly expectedRevision: TRevision;
  /**
   * True when this request repeats a write whose earlier outcome was unknown. The value and
   * expected revision are identical to that attempt, so a product can reuse its idempotency key
   * or read the server state first.
   */
  readonly afterUnknownOutcome: boolean;
  /** Aborts when the queue is cancelled or reset. */
  readonly signal: AbortSignal;
}

export interface RevisionedWriteQueueOptions<TScope, TDocument, TValue, TRevision, TConflict> {
  readonly initial: RevisionedWriteScope<TScope, TDocument, TRevision>;
  readonly write: (
    request: RevisionedWriteRequest<TScope, TDocument, TValue, TRevision>,
  ) => Promise<RevisionedWriteResult<TRevision, TConflict>>;
  /** Merges an older unsent value with a newer one. Defaults to keeping the newer value. */
  readonly coalesce?: (older: TValue, newer: TValue) => TValue;
  /** Cancels the queue permanently when it aborts, for example when its owner unmounts. */
  readonly signal?: AbortSignal;
}

export type RevisionedWriteStatus =
  'idle' | 'dirty' | 'writing' | 'failed' | 'unknown-outcome' | 'conflict' | 'cancelled';

export interface RevisionedWriteConflict<TRevision, TConflict> {
  readonly currentRevision: TRevision;
  readonly conflict: TConflict;
}

export interface UncertainRevisionedWrite<TValue, TRevision> {
  readonly value: TValue;
  readonly expectedRevision: TRevision;
}

export interface RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict> {
  readonly status: RevisionedWriteStatus;
  readonly scope: TScope;
  readonly document: TDocument;
  readonly acknowledgedRevision: TRevision;
  /** Local value the server has not seen, including a value it definitely did not apply. */
  readonly unsent: { readonly value: TValue } | null;
  /** A write the server may or may not have applied. */
  readonly uncertain: UncertainRevisionedWrite<TValue, TRevision> | null;
  readonly conflict: RevisionedWriteConflict<TRevision, TConflict> | null;
  /** The cause of the last failed or unknown write, or `null`. */
  readonly error: unknown;
}

export interface RevisionedWriteQueue<TScope, TDocument, TValue, TRevision, TConflict> {
  getSnapshot(): RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>;
  subscribe(listener: () => void): () => void;
  /** Records a local value. It is sent by the next `flush`, never by `enqueue` itself. */
  enqueue(value: TValue): void;
  /**
   * Sends unsent values one at a time until none is left or the queue pauses. Concurrent callers
   * share one drain. Resolves with the snapshot and never rejects.
   */
  flush(): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>>;
  /**
   * Resumes after `failed`, or replays the uncertain write after `unknown-outcome` with
   * `afterUnknownOutcome: true`. Does nothing during a conflict or after cancellation.
   */
  retry(): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>>;
  /**
   * Switches to another scope, document, or acknowledged revision. Aborts the active write,
   * drops unsent and uncertain values, and ignores every late completion.
   */
  reset(next: RevisionedWriteScope<TScope, TDocument, TRevision>): void;
  /** Aborts the active write, drops unsent values, and ignores late completions until `reset`. */
  cancel(): void;
}

type Pause = 'failed' | 'unknown-outcome' | 'conflict' | 'cancelled';

interface Attempt<TValue, TRevision> {
  readonly value: TValue;
  readonly expectedRevision: TRevision;
  readonly afterUnknownOutcome: boolean;
}

function keepNewer<TValue>(_older: TValue, newer: TValue): TValue {
  return newer;
}

class DefaultRevisionedWriteQueue<
  TScope,
  TDocument,
  TValue,
  TRevision,
  TConflict,
> implements RevisionedWriteQueue<TScope, TDocument, TValue, TRevision, TConflict> {
  readonly #write: RevisionedWriteQueueOptions<
    TScope,
    TDocument,
    TValue,
    TRevision,
    TConflict
  >['write'];
  readonly #coalesce: (older: TValue, newer: TValue) => TValue;
  readonly #hostSignal: AbortSignal | undefined;
  readonly #listeners = new Set<() => void>();
  #generation = 0;
  #abort = new AbortController();
  #scope: RevisionedWriteScope<TScope, TDocument, TRevision>;
  #acknowledgedRevision: TRevision;
  #unsent: { readonly value: TValue } | null = null;
  #uncertain: UncertainRevisionedWrite<TValue, TRevision> | null = null;
  #conflict: RevisionedWriteConflict<TRevision, TConflict> | null = null;
  #error: unknown = null;
  #writing = false;
  #pause: Pause | null = null;
  #drain: Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>> | null =
    null;
  #snapshot: RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>;

  constructor(
    options: RevisionedWriteQueueOptions<TScope, TDocument, TValue, TRevision, TConflict>,
  ) {
    this.#write = options.write;
    this.#coalesce = options.coalesce ?? keepNewer;
    this.#hostSignal = options.signal;
    this.#scope = options.initial;
    this.#acknowledgedRevision = options.initial.acknowledgedRevision;
    if (this.#hostSignal?.aborted === true) {
      this.#pause = 'cancelled';
    }
    this.#hostSignal?.addEventListener('abort', () => {
      this.cancel();
    });
    this.#snapshot = this.#buildSnapshot();
  }

  getSnapshot(): RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict> {
    return this.#snapshot;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  enqueue(value: TValue): void {
    if (this.#pause === 'cancelled') {
      return;
    }
    this.#unsent = this.#merge(this.#unsent, value);
    this.#publish();
  }

  flush(): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>> {
    if (this.#drain !== null) {
      return this.#drain;
    }
    const resumable = this.#pause === null || this.#pause === 'failed';
    if (!resumable || this.#unsent === null) {
      return Promise.resolve(this.#snapshot);
    }
    this.#pause = null;
    return this.#startDrain(null);
  }

  retry(): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>> {
    if (this.#drain !== null || this.#pause !== 'unknown-outcome' || this.#uncertain === null) {
      return this.flush();
    }
    const uncertain = this.#uncertain;
    this.#uncertain = null;
    this.#pause = null;
    return this.#startDrain({ ...uncertain, afterUnknownOutcome: true });
  }

  reset(next: RevisionedWriteScope<TScope, TDocument, TRevision>): void {
    this.#fence();
    this.#scope = next;
    this.#acknowledgedRevision = next.acknowledgedRevision;
    this.#pause = this.#hostSignal?.aborted === true ? 'cancelled' : null;
    this.#publish();
  }

  cancel(): void {
    if (this.#pause === 'cancelled') {
      return;
    }
    this.#fence();
    this.#pause = 'cancelled';
    this.#publish();
  }

  #fence(): void {
    this.#generation += 1;
    this.#abort.abort();
    this.#abort = new AbortController();
    this.#drain = null;
    this.#writing = false;
    this.#unsent = null;
    this.#uncertain = null;
    this.#conflict = null;
    this.#error = null;
  }

  #startDrain(
    first: Attempt<TValue, TRevision> | null,
  ): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>> {
    const drain = this.#runDrain(this.#generation, first);
    this.#drain = drain;
    return drain;
  }

  async #runDrain(
    generation: number,
    first: Attempt<TValue, TRevision> | null,
  ): Promise<RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict>> {
    try {
      let attempt = first ?? this.#takeUnsent();
      while (attempt !== null && (await this.#send(attempt, generation))) {
        attempt = this.#takeUnsent();
      }
    } finally {
      if (generation === this.#generation) {
        this.#drain = null;
      }
    }
    return this.#snapshot;
  }

  #takeUnsent(): Attempt<TValue, TRevision> | null {
    if (this.#pause !== null || this.#unsent === null) {
      return null;
    }
    const { value } = this.#unsent;
    this.#unsent = null;
    return { value, expectedRevision: this.#acknowledgedRevision, afterUnknownOutcome: false };
  }

  async #send(attempt: Attempt<TValue, TRevision>, generation: number): Promise<boolean> {
    this.#writing = true;
    this.#error = null;
    this.#publish();
    let result: RevisionedWriteResult<TRevision, TConflict>;
    try {
      result = await this.#write({
        scope: this.#scope.scope,
        document: this.#scope.document,
        value: attempt.value,
        expectedRevision: attempt.expectedRevision,
        afterUnknownOutcome: attempt.afterUnknownOutcome,
        signal: this.#abort.signal,
      });
    } catch (error) {
      if (generation === this.#generation) {
        this.#pauseUncertain(attempt, error);
      }
      return false;
    }
    if (generation !== this.#generation) {
      return false;
    }
    return this.#apply(attempt, result);
  }

  #apply(
    attempt: Attempt<TValue, TRevision>,
    result: RevisionedWriteResult<TRevision, TConflict>,
  ): boolean {
    this.#writing = false;
    switch (result.kind) {
      case 'accepted':
        this.#acknowledgedRevision = result.revision;
        this.#publish();
        return true;
      case 'rejected':
        this.#restoreUnapplied(attempt.value);
        this.#pause = 'failed';
        this.#error = result.error;
        this.#publish();
        return false;
      case 'conflict':
        this.#restoreUnapplied(attempt.value);
        this.#pause = 'conflict';
        this.#conflict = { currentRevision: result.currentRevision, conflict: result.conflict };
        this.#publish();
        return false;
    }
  }

  #pauseUncertain(attempt: Attempt<TValue, TRevision>, error: unknown): void {
    this.#writing = false;
    this.#uncertain = { value: attempt.value, expectedRevision: attempt.expectedRevision };
    this.#pause = 'unknown-outcome';
    this.#error = error;
    this.#publish();
  }

  #restoreUnapplied(value: TValue): void {
    this.#unsent =
      this.#unsent === null ? { value } : { value: this.#coalesce(value, this.#unsent.value) };
  }

  #merge(current: { readonly value: TValue } | null, value: TValue): { readonly value: TValue } {
    return current === null ? { value } : { value: this.#coalesce(current.value, value) };
  }

  #status(): RevisionedWriteStatus {
    if (this.#pause === 'cancelled') {
      return 'cancelled';
    }
    if (this.#writing) {
      return 'writing';
    }
    if (this.#pause !== null) {
      return this.#pause;
    }
    return this.#unsent === null ? 'idle' : 'dirty';
  }

  #buildSnapshot(): RevisionedWriteSnapshot<TScope, TDocument, TValue, TRevision, TConflict> {
    return {
      status: this.#status(),
      scope: this.#scope.scope,
      document: this.#scope.document,
      acknowledgedRevision: this.#acknowledgedRevision,
      unsent: this.#unsent,
      uncertain: this.#uncertain,
      conflict: this.#conflict,
      error: this.#error,
    };
  }

  #publish(): void {
    this.#snapshot = this.#buildSnapshot();
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

/** Creates a framework-free queue that serializes revision-checked writes to one document. */
export function createRevisionedWriteQueue<TScope, TDocument, TValue, TRevision, TConflict>(
  options: RevisionedWriteQueueOptions<TScope, TDocument, TValue, TRevision, TConflict>,
): RevisionedWriteQueue<TScope, TDocument, TValue, TRevision, TConflict> {
  return new DefaultRevisionedWriteQueue(options);
}

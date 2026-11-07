import type { MaybePromise } from './contracts.js';
import {
  isPersistenceIdentityMismatchError,
  PersistenceIdentityMismatchError,
  requiredIdentityPart,
} from './identity-error.js';
import {
  removeScopedPersistenceRegistryEntry,
  resolveScopedStore,
  type ResolveScopedStoreOptions,
  type ScopedPersistenceResolver,
} from './identity.js';

export interface ClosableScopedPersistence {
  close(): Promise<void>;
}

export interface OpenScopedPersistenceContext {
  readonly subject: string;
  readonly storeName: string;
}

export interface ScopedPersistencePartition<
  TPersistence extends ClosableScopedPersistence,
> extends OpenScopedPersistenceContext {
  readonly persistence: TPersistence;
}

export type ProfileErasureCleanup = 'pending' | 'complete' | 'failed';

export type ScopedPersistenceLifecycleState<TPersistence extends ClosableScopedPersistence> =
  | { readonly status: 'signed-out' }
  | { readonly status: 'initializing'; readonly subject: string }
  | {
      readonly status: 'ready';
      readonly partition: ScopedPersistencePartition<TPersistence>;
    }
  | {
      readonly status: 'blocked';
      readonly reason: 'identity-mismatch' | 'initialization-failed' | 'session-expired';
      readonly error: Error;
    }
  | {
      readonly status: 'blocked';
      readonly reason: 'profile-erased';
      readonly cleanup: ProfileErasureCleanup;
      readonly error: Error;
    };

export interface ScopedPersistenceLifecycleOptions<
  TPersistence extends ClosableScopedPersistence,
> extends Omit<ResolveScopedStoreOptions, 'subject'> {
  /** Overrides registry resolution while retaining lifecycle transition guarantees. */
  readonly resolveStore?: ScopedPersistenceResolver;
  readonly open: (context: OpenScopedPersistenceContext) => Promise<TPersistence>;
  /** Deletes a closed partition. Required by eraseActivePartition and handleProfileErased. */
  readonly erase?: (context: OpenScopedPersistenceContext) => Promise<void>;
  readonly resetUserScopedState: () => MaybePromise<void>;
}

/**
 * Serializes identity transitions, hides stale handles immediately, and only
 * publishes a partition after the product's open/migrate/hydrate hook resolves.
 */
export class ScopedPersistenceLifecycle<TPersistence extends ClosableScopedPersistence> {
  private activePartition: ScopedPersistencePartition<TPersistence> | undefined;
  private lastSelectedPartition: OpenScopedPersistenceContext | undefined;
  private erasedProfile:
    { readonly subject: string; cleanup: ProfileErasureCleanup; error: Error } | undefined;
  private transitionTail: Promise<void> = Promise.resolve();
  private revision = 0;
  private lifecycleState: ScopedPersistenceLifecycleState<TPersistence> = {
    status: 'signed-out',
  };

  public constructor(private readonly options: ScopedPersistenceLifecycleOptions<TPersistence>) {}

  public get state(): ScopedPersistenceLifecycleState<TPersistence> {
    return this.lifecycleState;
  }

  /** Returns no persistence while a transition is pending or blocked. */
  public current(): ScopedPersistencePartition<TPersistence> | undefined {
    return this.lifecycleState.status === 'ready' ? this.activePartition : undefined;
  }

  public selectSubject(
    subject: string,
  ): Promise<ScopedPersistencePartition<TPersistence> | undefined> {
    requiredIdentityPart(subject, 'Authenticated subject');
    if (this.lifecycleState.status === 'ready' && this.activePartition?.subject === subject) {
      return Promise.resolve(this.activePartition);
    }
    const revision = ++this.revision;
    this.lifecycleState = { status: 'initializing', subject };
    return this.enqueue(() => this.selectSubjectLocked(subject, revision));
  }

  public clear(): Promise<void> {
    const revision = ++this.revision;
    this.lifecycleState = { status: 'signed-out' };
    return this.enqueue(async () => {
      await this.closeAndReset();
      if (revision === this.revision) this.lifecycleState = { status: 'signed-out' };
    });
  }

  /** Closes and deletes the active partition, then removes its registry mapping. */
  public eraseActivePartition(): Promise<boolean> {
    const partition = this.current();
    if (partition === undefined) return Promise.resolve(false);
    const erase = this.options.erase;
    if (erase === undefined) {
      return Promise.reject(
        new Error('Active partition erasure requires a configured persistence erase hook.'),
      );
    }

    const revision = ++this.revision;
    this.activePartition = undefined;
    this.lifecycleState = { status: 'signed-out' };
    return this.enqueue(async () => {
      await this.closePartitionAndReset(partition);
      await this.erasePartition(partition, erase);
      if (revision === this.revision) this.lifecycleState = { status: 'signed-out' };
      return true;
    });
  }

  /** Erases the last selected partition, even after session cleanup closed it. */
  public handleProfileErased(): Promise<void> {
    const erase = this.options.erase;
    const revision = ++this.revision;
    let cleanup: ProfileErasureCleanup = 'pending';
    let error = new Error('The server profile was erased; sign in again to continue.');
    this.lifecycleState = { status: 'blocked', reason: 'profile-erased', cleanup, error };
    return this.enqueue(async () => {
      const partition = this.lastSelectedPartition;
      if (partition !== undefined)
        this.erasedProfile = { subject: partition.subject, cleanup, error };
      try {
        if (partition !== undefined) await this.closeAndReset();
        if (erase === undefined) {
          throw new Error('Active partition erasure requires a configured persistence erase hook.');
        }
        if (partition !== undefined) await this.erasePartition(partition, erase);
        cleanup = 'complete';
      } catch (cause) {
        cleanup = 'failed';
        error = cause instanceof Error ? cause : new Error('Local profile erasure failed.');
        throw cause;
      } finally {
        if (this.erasedProfile !== undefined && this.erasedProfile.subject === partition?.subject) {
          this.erasedProfile.cleanup = cleanup;
          this.erasedProfile.error = error;
        }
        if (revision === this.revision) {
          this.lifecycleState = { status: 'blocked', reason: 'profile-erased', cleanup, error };
        }
      }
    });
  }

  /** Terminal auth expiry closes the current partition and remains blocked. */
  public handleSessionExpired(): Promise<void> {
    const revision = ++this.revision;
    const error = new Error('The authenticated session expired; sign in again to continue.');
    this.lifecycleState = { status: 'blocked', reason: 'session-expired', error };
    return this.enqueue(async () => {
      await this.closeAndReset();
      if (revision === this.revision) {
        this.lifecycleState = { status: 'blocked', reason: 'session-expired', error };
      }
    });
  }

  /** Blocks an already-open partition after a server subject recheck fails. */
  public blockIdentityMismatch(error: PersistenceIdentityMismatchError): Promise<void> {
    const revision = ++this.revision;
    this.lifecycleState = { status: 'blocked', reason: 'identity-mismatch', error };
    return this.enqueue(async () => {
      await this.closeAndReset();
      if (revision === this.revision) {
        this.lifecycleState = { status: 'blocked', reason: 'identity-mismatch', error };
      }
    });
  }

  private enqueue<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const result = this.transitionTail.then(operation);
    this.transitionTail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  private async selectSubjectLocked(
    subject: string,
    revision: number,
  ): Promise<ScopedPersistencePartition<TPersistence> | undefined> {
    try {
      await this.closeAndReset();
      if (revision !== this.revision) return undefined;
      if (this.erasedProfile?.subject === subject) {
        this.lifecycleState = {
          status: 'blocked',
          reason: 'profile-erased',
          cleanup: this.erasedProfile.cleanup,
          error: this.erasedProfile.error,
        };
        return undefined;
      }
      if (this.lastSelectedPartition?.subject !== subject) this.lastSelectedPartition = undefined;
      const resolved =
        this.options.resolveStore === undefined
          ? await resolveScopedStore({ ...this.options, subject })
          : await this.options.resolveStore(subject);
      requiredIdentityPart(resolved.storeName, 'Resolved persistence store name');
      if (revision !== this.revision) return undefined;
      const context = {
        subject,
        storeName: resolved.storeName,
      };
      this.lastSelectedPartition = context;
      const persistence = await this.options.open(context);
      if (revision !== this.revision) {
        await persistence.close();
        await this.options.resetUserScopedState();
        return undefined;
      }
      const partition = { subject, storeName: resolved.storeName, persistence };
      this.activePartition = partition;
      this.lifecycleState = { status: 'ready', partition };
      return partition;
    } catch (cause) {
      if (revision === this.revision) {
        const error =
          cause instanceof Error ? cause : new Error('Local data initialization failed.');
        this.lifecycleState = {
          status: 'blocked',
          reason: isPersistenceIdentityMismatchError(cause)
            ? 'identity-mismatch'
            : 'initialization-failed',
          error,
        };
      }
      throw cause;
    }
  }

  private async erasePartition(
    partition: OpenScopedPersistenceContext,
    erase: NonNullable<ScopedPersistenceLifecycleOptions<TPersistence>['erase']>,
  ): Promise<void> {
    await erase({ subject: partition.subject, storeName: partition.storeName });
    await removeScopedPersistenceRegistryEntry({
      namespace: this.options.namespace,
      subject: partition.subject,
      registry: this.options.registry,
    });
    if (this.lastSelectedPartition?.subject === partition.subject) {
      this.lastSelectedPartition = undefined;
    }
  }

  private async closeAndReset(): Promise<void> {
    const partition = this.activePartition;
    this.activePartition = undefined;
    await this.closePartitionAndReset(partition);
  }

  private async closePartitionAndReset(
    partition: ScopedPersistencePartition<TPersistence> | undefined,
  ): Promise<void> {
    let closeFailure: unknown;
    try {
      await partition?.persistence.close();
    } catch (cause) {
      closeFailure = cause;
    } finally {
      await this.options.resetUserScopedState();
    }
    if (closeFailure !== undefined) {
      throw closeFailure instanceof Error
        ? closeFailure
        : new Error('Closing the active local-data partition failed.');
    }
  }
}

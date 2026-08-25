import type { MaybePromise } from './contracts.js';

const REGISTRY_VERSION = 1;
const SCOPED_STORE_PREFIX = 'baukit-scoped-v1-';
const SHA256_HEX_PATTERN = /^[0-9a-f]{64}$/u;

/** Product-selected behavior for a partition after logout. */
export type LocalDataRetention = 'retain' | 'delete' | 'quarantine';

/** Registry persistence kept outside the user-owned domain database. */
export interface ScopedPersistenceRegistryStore {
  read(): Promise<string | null>;
  write(serialized: string): Promise<void>;
}

/** A digest seam for runtimes that install their Web Crypto implementation explicitly. */
export type ScopedPersistenceDigest = (value: string) => Promise<string>;

export type LegacyStoreOwnership = 'claimable' | 'current-subject' | 'other-subject' | 'ambiguous';

export type LegacyStoreInspection =
  { readonly exists: false } | { readonly exists: true; readonly ownership: LegacyStoreOwnership };

export interface ResolveScopedStoreOptions {
  readonly namespace: string;
  readonly subject: string;
  readonly registry: ScopedPersistenceRegistryStore;
  readonly digest?: ScopedPersistenceDigest;
  readonly inspectLegacy?: (subject: string) => Promise<LegacyStoreInspection>;
  readonly legacyStoreName?: string;
}

export interface ResolvedScopedStore {
  readonly storeName: string;
  readonly claimedLegacy: boolean;
}

/** Product compatibility resolver used when older ownership metadata already exists. */
export type ScopedPersistenceResolver = (subject: string) => MaybePromise<ResolvedScopedStore>;

/** Stable blocking failure for corrupt ownership metadata or a subject mismatch. */
export class PersistenceIdentityMismatchError extends Error {
  public override readonly name = 'PersistenceIdentityMismatchError';
  public readonly code = 'persistence_identity_mismatch' as const;

  public constructor(
    message = 'Local data belongs to a different authenticated identity.',
    options?: ErrorOptions,
  ) {
    super(message, options);
  }
}

export function isPersistenceIdentityMismatchError(
  cause: unknown,
): cause is PersistenceIdentityMismatchError {
  return (
    cause instanceof PersistenceIdentityMismatchError ||
    (typeof cause === 'object' &&
      cause !== null &&
      Reflect.get(cause, 'code') === 'persistence_identity_mismatch')
  );
}

interface RegistryEntry {
  readonly subject: string;
  readonly storeName: string;
  readonly claimedLegacy: boolean;
}

interface RegistryNamespace {
  readonly namespace: string;
  legacyClaimedBySubject: string | null;
  readonly subjects: RegistryEntry[];
}

interface PersistenceRegistry {
  readonly version: typeof REGISTRY_VERSION;
  readonly namespaces: RegistryNamespace[];
}

/** Small deterministic registry adapter for tests and non-persistent products. */
export class InMemoryScopedPersistenceRegistryStore implements ScopedPersistenceRegistryStore {
  public constructor(private serialized: string | null = null) {}

  public read(): Promise<string | null> {
    return Promise.resolve(this.serialized);
  }

  public write(serialized: string): Promise<void> {
    this.serialized = serialized;
    return Promise.resolve();
  }
}

interface SubtleCryptoLike {
  digest(algorithm: string, data: Uint8Array): Promise<ArrayBuffer>;
}

function webCryptoSubtle(): SubtleCryptoLike {
  const cryptoValue: unknown = Reflect.get(globalThis, 'crypto');
  const subtle: unknown =
    typeof cryptoValue === 'object' && cryptoValue !== null
      ? Reflect.get(cryptoValue, 'subtle')
      : undefined;
  if (
    typeof subtle !== 'object' ||
    subtle === null ||
    typeof Reflect.get(subtle, 'digest') !== 'function'
  ) {
    throw new PersistenceIdentityMismatchError(
      'A Web Crypto SHA-256 implementation is required before local data can be opened.',
    );
  }
  return subtle as SubtleCryptoLike;
}

function utf8(value: string): Uint8Array {
  const bytes: number[] = [];
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint === undefined) continue;
    if (codePoint <= 0x7f) {
      bytes.push(codePoint);
    } else if (codePoint <= 0x7ff) {
      bytes.push(0xc0 | (codePoint >> 6), 0x80 | (codePoint & 0x3f));
    } else if (codePoint <= 0xffff) {
      bytes.push(
        0xe0 | (codePoint >> 12),
        0x80 | ((codePoint >> 6) & 0x3f),
        0x80 | (codePoint & 0x3f),
      );
    } else {
      bytes.push(
        0xf0 | (codePoint >> 18),
        0x80 | ((codePoint >> 12) & 0x3f),
        0x80 | ((codePoint >> 6) & 0x3f),
        0x80 | (codePoint & 0x3f),
      );
    }
  }
  return new Uint8Array(bytes);
}

function hexadecimal(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function sha256(value: string): Promise<string> {
  const digest = await webCryptoSubtle().digest('SHA-256', utf8(value));
  return hexadecimal(new Uint8Array(digest));
}

function requiredIdentityPart(value: string, label: string): string {
  if (value.trim().length === 0) {
    throw new PersistenceIdentityMismatchError(`${label} must not be empty.`);
  }
  return value;
}

function canonicalIdentity(namespace: string, subject: string): string {
  const namespaceBytes = utf8(namespace).byteLength;
  const subjectBytes = utf8(subject).byteLength;
  return `baukit-scoped-persistence:v1:${String(namespaceBytes)}:${namespace}:${String(subjectBytes)}:${subject}`;
}

/** Derives an opaque, stable name without disclosing the namespace or OIDC subject. */
export async function deriveScopedStoreName(
  namespace: string,
  subject: string,
  digest: ScopedPersistenceDigest = sha256,
): Promise<string> {
  requiredIdentityPart(namespace, 'Persistence namespace');
  requiredIdentityPart(subject, 'Authenticated subject');
  let hash: string;
  try {
    hash = (await digest(canonicalIdentity(namespace, subject))).toLowerCase();
  } catch (cause) {
    if (isPersistenceIdentityMismatchError(cause)) throw cause;
    throw new PersistenceIdentityMismatchError('The local-data identity digest failed.', { cause });
  }
  if (!SHA256_HEX_PATTERN.test(hash)) {
    throw new PersistenceIdentityMismatchError('The local-data identity digest is invalid.');
  }
  return `${SCOPED_STORE_PREFIX}${hash}`;
}

function emptyRegistry(): PersistenceRegistry {
  return { version: REGISTRY_VERSION, namespaces: [] };
}

function corruptRegistry(cause?: unknown): PersistenceIdentityMismatchError {
  return new PersistenceIdentityMismatchError(
    'Local account ownership metadata is invalid; local data was not opened.',
    cause === undefined ? undefined : { cause },
  );
}

function parseRegistry(serialized: string | null): PersistenceRegistry {
  if (serialized === null) return emptyRegistry();
  try {
    const value: unknown = JSON.parse(serialized);
    if (
      typeof value !== 'object' ||
      value === null ||
      Reflect.get(value, 'version') !== REGISTRY_VERSION ||
      !Array.isArray(Reflect.get(value, 'namespaces'))
    ) {
      throw new TypeError('invalid registry root');
    }
    const namespaces = Reflect.get(value, 'namespaces') as unknown[];
    const namespaceNames = new Set<string>();
    const storeNames = new Set<string>();
    for (const candidate of namespaces) {
      validateRegistryNamespace(candidate, namespaceNames, storeNames);
    }
    return value as PersistenceRegistry;
  } catch (cause) {
    if (isPersistenceIdentityMismatchError(cause)) throw cause;
    throw corruptRegistry(cause);
  }
}

function validateRegistryNamespace(
  candidate: unknown,
  names: Set<string>,
  storeNames: Set<string>,
): void {
  if (typeof candidate !== 'object' || candidate === null) {
    throw new TypeError('invalid namespace entry');
  }
  const namespace: unknown = Reflect.get(candidate, 'namespace');
  const legacyClaimedBySubject: unknown = Reflect.get(candidate, 'legacyClaimedBySubject');
  const subjects: unknown = Reflect.get(candidate, 'subjects');
  if (
    typeof namespace !== 'string' ||
    namespace.trim().length === 0 ||
    names.has(namespace) ||
    (legacyClaimedBySubject !== null &&
      (typeof legacyClaimedBySubject !== 'string' || legacyClaimedBySubject.trim().length === 0)) ||
    !Array.isArray(subjects)
  ) {
    throw new TypeError('invalid namespace metadata');
  }
  names.add(namespace);

  const subjectNames = new Set<string>();
  const claimed: RegistryEntry[] = [];
  for (const entry of subjects) {
    if (typeof entry !== 'object' || entry === null) {
      throw new TypeError('invalid subject entry');
    }
    const subject: unknown = Reflect.get(entry, 'subject');
    const storeName: unknown = Reflect.get(entry, 'storeName');
    const claimedLegacy: unknown = Reflect.get(entry, 'claimedLegacy');
    if (
      typeof subject !== 'string' ||
      subject.trim().length === 0 ||
      subjectNames.has(subject) ||
      typeof storeName !== 'string' ||
      storeName.trim().length === 0 ||
      typeof claimedLegacy !== 'boolean' ||
      storeNames.has(storeName)
    ) {
      throw new TypeError('invalid subject metadata');
    }
    subjectNames.add(subject);
    storeNames.add(storeName);
    if (claimedLegacy) claimed.push(entry as RegistryEntry);
  }
  if (
    (legacyClaimedBySubject === null && claimed.length !== 0) ||
    (legacyClaimedBySubject !== null &&
      (claimed.length > 1 ||
        (claimed.length === 1 && claimed[0]?.subject !== legacyClaimedBySubject)))
  ) {
    throw new TypeError('inconsistent legacy ownership metadata');
  }
}

const registryQueues = new WeakMap<object, Promise<void>>();

function withRegistryLock<TResult>(
  registry: ScopedPersistenceRegistryStore,
  operation: () => Promise<TResult>,
): Promise<TResult> {
  const previous = registryQueues.get(registry) ?? Promise.resolve();
  const result = previous.then(operation);
  registryQueues.set(
    registry,
    result.then(
      () => undefined,
      () => undefined,
    ),
  );
  return result;
}

function findNamespace(
  registry: PersistenceRegistry,
  namespace: string,
): RegistryNamespace | undefined {
  return registry.namespaces.find((candidate) => candidate.namespace === namespace);
}

function canClaimLegacy(inspection: LegacyStoreInspection): boolean {
  return (
    inspection.exists &&
    (inspection.ownership === 'claimable' || inspection.ownership === 'current-subject')
  );
}

async function resolveScopedStoreLocked(
  options: ResolveScopedStoreOptions,
): Promise<ResolvedScopedStore> {
  const namespaceName = requiredIdentityPart(options.namespace, 'Persistence namespace');
  const subject = requiredIdentityPart(options.subject, 'Authenticated subject');
  const registry = parseRegistry(await options.registry.read());
  let namespace = findNamespace(registry, namespaceName);
  const existing = namespace?.subjects.find((entry) => entry.subject === subject);
  if (existing !== undefined) {
    if (existing.claimedLegacy) {
      if (options.legacyStoreName === undefined || existing.storeName !== options.legacyStoreName) {
        throw corruptRegistry();
      }
    } else if (
      existing.storeName !==
      (await deriveScopedStoreName(namespaceName, subject, options.digest ?? sha256))
    ) {
      throw corruptRegistry();
    }
    return { storeName: existing.storeName, claimedLegacy: existing.claimedLegacy };
  }

  if (namespace === undefined) {
    namespace = { namespace: namespaceName, legacyClaimedBySubject: null, subjects: [] };
    registry.namespaces.push(namespace);
  }

  let storeName = await deriveScopedStoreName(namespaceName, subject, options.digest ?? sha256);
  let claimedLegacy = false;
  if (
    namespace.legacyClaimedBySubject === null &&
    options.legacyStoreName !== undefined &&
    options.inspectLegacy !== undefined
  ) {
    requiredIdentityPart(options.legacyStoreName, 'Legacy store name');
    const inspection = await options.inspectLegacy(subject);
    if (canClaimLegacy(inspection)) {
      storeName = options.legacyStoreName;
      claimedLegacy = true;
      namespace.legacyClaimedBySubject = subject;
    }
  }

  const storeAlreadyRegistered = registry.namespaces.some((candidate) =>
    candidate.subjects.some((entry) => entry.storeName === storeName),
  );
  if (storeAlreadyRegistered) throw corruptRegistry();

  namespace.subjects.push({ subject, storeName, claimedLegacy });
  await options.registry.write(JSON.stringify(registry));
  return { storeName, claimedLegacy };
}

/** Resolves or atomically records one authenticated subject's local partition. */
export function resolveScopedStore(
  options: ResolveScopedStoreOptions,
): Promise<ResolvedScopedStore> {
  return withRegistryLock(options.registry, () => resolveScopedStoreLocked(options));
}

export interface RemoveScopedPersistenceRegistryEntryOptions {
  readonly namespace: string;
  readonly subject: string;
  readonly registry: ScopedPersistenceRegistryStore;
}

async function removeScopedPersistenceRegistryEntryLocked(
  options: RemoveScopedPersistenceRegistryEntryOptions,
): Promise<boolean> {
  const namespaceName = requiredIdentityPart(options.namespace, 'Persistence namespace');
  const subject = requiredIdentityPart(options.subject, 'Authenticated subject');
  const registry = parseRegistry(await options.registry.read());
  const namespace = findNamespace(registry, namespaceName);
  if (namespace === undefined) return false;

  const entryIndex = namespace.subjects.findIndex((entry) => entry.subject === subject);
  if (entryIndex === -1) return false;
  namespace.subjects.splice(entryIndex, 1);
  if (namespace.subjects.length === 0 && namespace.legacyClaimedBySubject === null) {
    const namespaceIndex = registry.namespaces.indexOf(namespace);
    registry.namespaces.splice(namespaceIndex, 1);
  }
  await options.registry.write(JSON.stringify(registry));
  return true;
}

/** Removes one subject mapping without exposing serialized registry metadata. */
export function removeScopedPersistenceRegistryEntry(
  options: RemoveScopedPersistenceRegistryEntryOptions,
): Promise<boolean> {
  return withRegistryLock(options.registry, () =>
    removeScopedPersistenceRegistryEntryLocked(options),
  );
}

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
    };

export interface ScopedPersistenceLifecycleOptions<
  TPersistence extends ClosableScopedPersistence,
> extends Omit<ResolveScopedStoreOptions, 'subject'> {
  /** Overrides registry resolution while retaining lifecycle transition guarantees. */
  readonly resolveStore?: ScopedPersistenceResolver;
  readonly open: (context: OpenScopedPersistenceContext) => Promise<TPersistence>;
  /** Deletes a closed persistence partition. Required by eraseActivePartition. */
  readonly erase?: (context: OpenScopedPersistenceContext) => Promise<void>;
  readonly resetUserScopedState: () => MaybePromise<void>;
}

/**
 * Serializes identity transitions, hides stale handles immediately, and only
 * publishes a partition after the product's open/migrate/hydrate hook resolves.
 */
export class ScopedPersistenceLifecycle<TPersistence extends ClosableScopedPersistence> {
  private activePartition: ScopedPersistencePartition<TPersistence> | undefined;
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
      await erase({ subject: partition.subject, storeName: partition.storeName });
      await removeScopedPersistenceRegistryEntry({
        namespace: this.options.namespace,
        subject: partition.subject,
        registry: this.options.registry,
      });
      if (revision === this.revision) this.lifecycleState = { status: 'signed-out' };
      return true;
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
      const resolved =
        this.options.resolveStore === undefined
          ? await resolveScopedStore({ ...this.options, subject })
          : await this.options.resolveStore(subject);
      requiredIdentityPart(resolved.storeName, 'Resolved persistence store name');
      if (revision !== this.revision) return undefined;
      const persistence = await this.options.open({
        subject,
        storeName: resolved.storeName,
      });
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

export interface RecheckServerSubjectOptions<TResult> {
  readonly partitionSubject: string;
  readonly readServerSubject: () => Promise<string>;
  readonly adopt: () => MaybePromise<TResult>;
}

/** Performs the final subject check before sync adoption or an outbox push. */
export async function recheckServerSubjectBeforeSyncAdoption<TResult>(
  options: RecheckServerSubjectOptions<TResult>,
): Promise<TResult> {
  requiredIdentityPart(options.partitionSubject, 'Partition subject');
  const serverSubject = await options.readServerSubject();
  if (serverSubject.trim().length === 0 || serverSubject !== options.partitionSubject) {
    throw new PersistenceIdentityMismatchError(
      'The server identity does not match the active local-data partition.',
    );
  }
  return options.adopt();
}

import type { MaybePromise } from './contracts.js';
import {
  isPersistenceIdentityMismatchError,
  PersistenceIdentityMismatchError,
  requiredIdentityPart,
} from './identity-error.js';

export { isPersistenceIdentityMismatchError, PersistenceIdentityMismatchError };

const REGISTRY_VERSION = 1;
const SCOPED_STORE_PREFIX = 'baukit-scoped-v1-';
const SHA256_HEX_PATTERN = /^[0-9a-f]{64}$/u;

/** Product-selected behavior for a partition after logout. */
export type LocalDataRetention = 'retain' | 'delete' | 'quarantine';

/** Registry persistence kept outside the user-owned domain database. */
export interface ScopedPersistenceRegistryStore {
  read(): Promise<string | null>;
  write(serialized: string): Promise<void>;
  /** Writes `next` only if the stored document still equals `expected`. Preferred over `write`. */
  compareAndSet?(expected: string | null, next: string): Promise<boolean>;
  /** Cross-context mutual exclusion, such as Web Locks, held around each registry update. */
  withLock?<TResult>(operation: () => Promise<TResult>): Promise<TResult>;
}

const MAX_REGISTRY_UPDATE_ATTEMPTS = 8;

/** Runs `operation` under a Web Lock named `name` when `navigator.locks` exists. */
export function withWebLock<TResult>(
  name: string,
  operation: () => Promise<TResult>,
): Promise<TResult> {
  const navigatorValue: unknown = Reflect.get(globalThis, 'navigator');
  const locks: unknown =
    typeof navigatorValue === 'object' && navigatorValue !== null
      ? Reflect.get(navigatorValue, 'locks')
      : undefined;
  if (typeof locks !== 'object' || locks === null) return operation();
  const request = Reflect.get(locks, 'request') as unknown;
  if (typeof request !== 'function') return operation();
  return (
    request as (name: string, callback: () => Promise<TResult>) => Promise<TResult>
  ).call(locks, name, operation);
}

/** A digest seam for runtimes that install their Web Crypto implementation explicitly. */
export type ScopedPersistenceDigest = (value: string) => Promise<string>;

export interface ResolveScopedStoreOptions {
  readonly namespace: string;
  readonly subject: string;
  readonly registry: ScopedPersistenceRegistryStore;
  readonly digest?: ScopedPersistenceDigest;
}

export interface ResolvedScopedStore {
  readonly storeName: string;
}

/** Product-owned resolver that replaces the derived scoped store name. */
export type ScopedPersistenceResolver = (subject: string) => MaybePromise<ResolvedScopedStore>;

interface RegistryEntry {
  readonly subject: string;
  readonly storeName: string;
}

interface RegistryNamespace {
  readonly namespace: string;
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
  const subjects: unknown = Reflect.get(candidate, 'subjects');
  if (
    typeof namespace !== 'string' ||
    namespace.trim().length === 0 ||
    names.has(namespace) ||
    !Array.isArray(subjects)
  ) {
    throw new TypeError('invalid namespace metadata');
  }
  names.add(namespace);

  const subjectNames = new Set<string>();
  for (const entry of subjects) {
    validateRegistrySubject(entry, subjectNames, storeNames);
  }
}

function isBlank(value: unknown): boolean {
  return typeof value !== 'string' || value.trim().length === 0;
}

function validateRegistrySubject(
  entry: unknown,
  subjectNames: Set<string>,
  storeNames: Set<string>,
): void {
  if (typeof entry !== 'object' || entry === null) {
    throw new TypeError('invalid subject entry');
  }
  const subject: unknown = Reflect.get(entry, 'subject');
  const storeName: unknown = Reflect.get(entry, 'storeName');
  if (
    typeof subject !== 'string' ||
    typeof storeName !== 'string' ||
    isBlank(subject) ||
    isBlank(storeName) ||
    subjectNames.has(subject) ||
    storeNames.has(storeName)
  ) {
    throw new TypeError('invalid subject metadata');
  }
  subjectNames.add(subject);
  storeNames.add(storeName);
}

const registryQueues = new WeakMap<object, Promise<void>>();

function withRegistryLock<TResult>(
  registry: ScopedPersistenceRegistryStore,
  operation: () => Promise<TResult>,
): Promise<TResult> {
  const previous = registryQueues.get(registry) ?? Promise.resolve();
  const locked = (): Promise<TResult> =>
    registry.withLock === undefined ? operation() : registry.withLock(operation);
  const result = previous.then(locked);
  registryQueues.set(
    registry,
    result.then(
      () => undefined,
      () => undefined,
    ),
  );
  return result;
}

interface RegistryUpdate<TResult> {
  readonly result: TResult;
  readonly changed: boolean;
}

/** Re-reads, mutates and writes, retrying when another writer changed the document. */
async function updateRegistry<TResult>(
  store: ScopedPersistenceRegistryStore,
  mutate: (registry: PersistenceRegistry) => RegistryUpdate<TResult>,
): Promise<TResult> {
  for (let attempt = 0; attempt < MAX_REGISTRY_UPDATE_ATTEMPTS; attempt += 1) {
    const before = await store.read();
    const registry = parseRegistry(before);
    const { result, changed } = mutate(registry);
    if (!changed) return result;
    const next = JSON.stringify(registry);
    if (store.compareAndSet === undefined) {
      await store.write(next);
      return result;
    }
    if (await store.compareAndSet(before, next)) return result;
  }
  throw new PersistenceIdentityMismatchError(
    'Local account ownership metadata kept changing; local data was not opened.',
  );
}

function findNamespace(
  registry: PersistenceRegistry,
  namespace: string,
): RegistryNamespace | undefined {
  return registry.namespaces.find((candidate) => candidate.namespace === namespace);
}

async function resolveScopedStoreLocked(
  options: ResolveScopedStoreOptions,
): Promise<ResolvedScopedStore> {
  const namespaceName = requiredIdentityPart(options.namespace, 'Persistence namespace');
  const subject = requiredIdentityPart(options.subject, 'Authenticated subject');
  parseRegistry(await options.registry.read());
  const storeName = await deriveScopedStoreName(namespaceName, subject, options.digest ?? sha256);
  return updateRegistry(options.registry, (registry) => {
    let namespace = findNamespace(registry, namespaceName);
    const existing = namespace?.subjects.find((entry) => entry.subject === subject);
    if (existing !== undefined) {
      if (existing.storeName !== storeName) throw corruptRegistry();
      return { result: { storeName }, changed: false };
    }
    if (namespace === undefined) {
      namespace = { namespace: namespaceName, subjects: [] };
      registry.namespaces.push(namespace);
    }
    const storeAlreadyRegistered = registry.namespaces.some((candidate) =>
      candidate.subjects.some((entry) => entry.storeName === storeName),
    );
    if (storeAlreadyRegistered) throw corruptRegistry();
    namespace.subjects.push({ subject, storeName });
    return { result: { storeName }, changed: true };
  });
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

function removeScopedPersistenceRegistryEntryLocked(
  options: RemoveScopedPersistenceRegistryEntryOptions,
): Promise<boolean> {
  const namespaceName = requiredIdentityPart(options.namespace, 'Persistence namespace');
  const subject = requiredIdentityPart(options.subject, 'Authenticated subject');
  return updateRegistry(options.registry, (registry) => {
    const namespace = findNamespace(registry, namespaceName);
    const entryIndex = namespace?.subjects.findIndex((entry) => entry.subject === subject) ?? -1;
    if (namespace === undefined || entryIndex === -1) return { result: false, changed: false };
    namespace.subjects.splice(entryIndex, 1);
    if (namespace.subjects.length === 0) {
      registry.namespaces.splice(registry.namespaces.indexOf(namespace), 1);
    }
    return { result: true, changed: true };
  });
}

/** Removes one subject mapping without exposing serialized registry metadata. */
export function removeScopedPersistenceRegistryEntry(
  options: RemoveScopedPersistenceRegistryEntryOptions,
): Promise<boolean> {
  return withRegistryLock(options.registry, () =>
    removeScopedPersistenceRegistryEntryLocked(options),
  );
}

/** A source of transition revisions, such as `ScopedPersistenceLifecycle`. */
export interface TransitionRevisionSource {
  readonly transitionRevision: number;
}

export interface SyncAdoptionFence {
  /** Aborts with the caller's signal or once `owned()` reports a lost fence. */
  readonly signal: AbortSignal;
  /** Call before each request; false (and an aborted signal) once ownership moved. */
  owned(): boolean;
}

export interface RecheckServerSubjectOptions<TResult> {
  readonly partitionSubject: string;
  readonly readServerSubject: () => Promise<string>;
  readonly adopt: (fence: SyncAdoptionFence) => MaybePromise<TResult>;
  /** Fails the adoption when the lifecycle transitioned after the recheck started. */
  readonly lifecycle?: TransitionRevisionSource;
  /** Subject whose token the next request would carry; must equal the partition's. */
  readonly tokenSubject?: () => string | undefined;
  readonly signal?: AbortSignal;
}

function identityMoved(): PersistenceIdentityMismatchError {
  return new PersistenceIdentityMismatchError(
    'The server identity does not match the active local-data partition.',
  );
}

/**
 * Performs the final subject check before sync adoption or an outbox push, then
 * hands `adopt` a fence bound to the lifecycle revision and the token subject.
 */
export async function recheckServerSubjectBeforeSyncAdoption<TResult>(
  options: RecheckServerSubjectOptions<TResult>,
): Promise<TResult> {
  requiredIdentityPart(options.partitionSubject, 'Partition subject');
  const revision = options.lifecycle?.transitionRevision;
  const serverSubject = await options.readServerSubject();
  if (serverSubject.trim().length === 0 || serverSubject !== options.partitionSubject) {
    throw identityMoved();
  }
  options.signal?.throwIfAborted();
  const controller = new AbortController();
  const stillOwned = (): boolean =>
    options.lifecycle?.transitionRevision === revision &&
    (options.tokenSubject === undefined || options.tokenSubject() === options.partitionSubject);
  if (!stillOwned()) throw identityMoved();
  const forward = (): void => {
    controller.abort(options.signal?.reason);
  };
  options.signal?.addEventListener('abort', forward, { once: true });
  const fence: SyncAdoptionFence = {
    signal: controller.signal,
    owned: () => {
      if (!controller.signal.aborted && !stillOwned()) controller.abort(identityMoved());
      return !controller.signal.aborted;
    },
  };
  try {
    return await options.adopt(fence);
  } finally {
    options.signal?.removeEventListener('abort', forward);
  }
}

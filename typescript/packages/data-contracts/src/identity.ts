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

async function resolveScopedStoreLocked(
  options: ResolveScopedStoreOptions,
): Promise<ResolvedScopedStore> {
  const namespaceName = requiredIdentityPart(options.namespace, 'Persistence namespace');
  const subject = requiredIdentityPart(options.subject, 'Authenticated subject');
  const registry = parseRegistry(await options.registry.read());
  let namespace = findNamespace(registry, namespaceName);
  const storeName = await deriveScopedStoreName(namespaceName, subject, options.digest ?? sha256);
  const existing = namespace?.subjects.find((entry) => entry.subject === subject);
  if (existing !== undefined) {
    if (existing.storeName !== storeName) throw corruptRegistry();
    return { storeName };
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
  await options.registry.write(JSON.stringify(registry));
  return { storeName };
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
  if (namespace.subjects.length === 0) {
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

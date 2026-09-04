/// <reference types="node" />

import { constants, type Stats } from 'node:fs';
import { chmod, lstat, mkdir, open, rename, unlink, type FileHandle } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, parse, resolve, sep } from 'node:path';
import { randomUUID } from 'node:crypto';

import { AuthNodeError } from './errors.js';

const CACHE_VERSION = 1;
const CACHE_MAX_BYTES = 256 * 1024;
const DEFAULT_LOCK_TIMEOUT_MS = 10_000;
const DEFAULT_LOCK_RETRY_MS = 50;
const PRIVATE_FILE_MODE = 0o600;
const PRIVATE_DIRECTORY_MODE = 0o700;
const UNSAFE_MODE_BITS = 0o077;
const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u;

export interface CachedTokenProfile {
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly tokenType: string;
  readonly expiresAt: number;
  readonly issuer: string;
  readonly clientId: string;
  readonly scopes: readonly string[];
  readonly audience?: string;
}

interface CacheDocument {
  readonly version: typeof CACHE_VERSION;
  readonly namespace: string;
  readonly profiles: Readonly<Record<string, CachedTokenProfile>>;
}

export interface TokenCacheTransaction {
  read(profile: string): Promise<CachedTokenProfile | undefined>;
  write(profile: string, value: CachedTokenProfile): Promise<void>;
  remove(profile: string): Promise<boolean>;
}

export interface NodeTokenCacheOptions {
  readonly namespace: string;
  readonly path?: string;
  readonly platform?: NodeJS.Platform;
  readonly now?: () => number;
  readonly sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
  readonly lockTimeoutMs?: number;
  readonly lockRetryMs?: number;
}

export function defaultTokenCachePath(
  namespace: string,
  environment: Readonly<Record<string, string | undefined>> = process.env,
): string {
  validateName(namespace, 'cache namespace');
  const configuredHome = environment['XDG_CONFIG_HOME']?.trim();
  const configHome =
    configuredHome === undefined || configuredHome.length === 0
      ? join(homedir(), '.config')
      : configuredHome;
  return join(configHome, namespace, 'tokens.json');
}

/** A versioned profile cache that serializes mutations with an adjacent lock file. */
export class NodeTokenCache {
  public readonly path: string;
  public readonly namespace: string;
  readonly #platform: NodeJS.Platform;
  readonly #now: () => number;
  readonly #sleep: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
  readonly #lockTimeoutMs: number;
  readonly #lockRetryMs: number;

  public constructor(options: NodeTokenCacheOptions) {
    validateName(options.namespace, 'cache namespace');
    this.namespace = options.namespace;
    this.path = resolve(options.path ?? defaultTokenCachePath(options.namespace));
    this.#platform = options.platform ?? process.platform;
    this.#now = options.now ?? Date.now;
    this.#sleep = options.sleep ?? abortableDelay;
    this.#lockTimeoutMs = positiveInteger(
      options.lockTimeoutMs ?? DEFAULT_LOCK_TIMEOUT_MS,
      'cache lock timeout',
    );
    this.#lockRetryMs = positiveInteger(
      options.lockRetryMs ?? DEFAULT_LOCK_RETRY_MS,
      'cache lock retry interval',
    );
  }

  public async read(profile = 'default'): Promise<CachedTokenProfile | undefined> {
    validateName(profile, 'cache profile');
    const document = await this.#readDocument();
    return document === undefined ? undefined : ownProfile(document.profiles, profile);
  }

  public write(profile: string, value: CachedTokenProfile, signal?: AbortSignal): Promise<void> {
    return this.withLock(async (transaction) => {
      await transaction.write(profile, value);
    }, signal);
  }

  public remove(profile = 'default', signal?: AbortSignal): Promise<boolean> {
    return this.withLock((transaction) => transaction.remove(profile), signal);
  }

  public async withLock<T>(
    action: (transaction: TokenCacheTransaction) => Promise<T>,
    signal?: AbortSignal,
  ): Promise<T> {
    throwIfAborted(signal);
    const lock = await this.#acquireLock(signal);
    try {
      const transaction: TokenCacheTransaction = {
        read: (profile) => this.#readProfile(profile),
        write: (profile, value) => this.#writeProfile(profile, value),
        remove: (profile) => this.#removeProfile(profile),
      };
      return await action(transaction);
    } finally {
      await this.#releaseLock(lock);
    }
  }

  async #readProfile(profile: string): Promise<CachedTokenProfile | undefined> {
    validateName(profile, 'cache profile');
    const document = await this.#readDocument();
    return document === undefined ? undefined : ownProfile(document.profiles, profile);
  }

  async #writeProfile(profile: string, value: CachedTokenProfile): Promise<void> {
    validateName(profile, 'cache profile');
    validateProfile(value);
    const current = await this.#readDocument();
    const profiles = { ...(current?.profiles ?? {}), [profile]: value };
    await this.#replaceDocument({ version: CACHE_VERSION, namespace: this.namespace, profiles });
  }

  async #removeProfile(profile: string): Promise<boolean> {
    validateName(profile, 'cache profile');
    const current = await this.#readDocument();
    if (current === undefined || ownProfile(current.profiles, profile) === undefined) return false;
    const profiles = Object.fromEntries(
      Object.entries(current.profiles).filter(([name]) => name !== profile),
    );
    await this.#replaceDocument({ version: CACHE_VERSION, namespace: this.namespace, profiles });
    return true;
  }

  async #readDocument(): Promise<CacheDocument | undefined> {
    await assertNoSymlink(this.path, true);
    let handle: FileHandle;
    try {
      handle = await open(this.path, constants.O_RDONLY | constants.O_NOFOLLOW);
    } catch (error) {
      if (nodeError(error, 'ENOENT')) return undefined;
      throw cacheIoError(error, 'cache_read_failed');
    }
    try {
      const metadata = await handle.stat();
      validateCacheFile(metadata, this.#platform);
      if (metadata.size > CACHE_MAX_BYTES) throw new AuthNodeError('cache_corrupt');
      const contents = await handle.readFile({ encoding: 'utf8' });
      if (Buffer.byteLength(contents) > CACHE_MAX_BYTES) {
        throw new AuthNodeError('cache_corrupt');
      }
      return parseDocument(contents, this.namespace);
    } catch (error) {
      if (error instanceof AuthNodeError) throw error;
      throw cacheIoError(error, 'cache_read_failed');
    } finally {
      await handle.close().catch(() => undefined);
    }
  }

  async #replaceDocument(document: CacheDocument): Promise<void> {
    const directory = dirname(this.path);
    await preparePrivateDirectory(directory, this.#platform);
    await assertNoSymlink(this.path, true);
    const temporaryPath = `${this.path}.${String(process.pid)}.${randomUUID()}.tmp`;
    let handle: FileHandle | undefined;
    try {
      handle = await open(temporaryPath, 'wx', PRIVATE_FILE_MODE);
      await handle.writeFile(`${JSON.stringify(document, null, 2)}\n`, 'utf8');
      await handle.sync();
      await handle.close();
      handle = undefined;
      if (this.#platform !== 'win32') await chmod(temporaryPath, PRIVATE_FILE_MODE);
      await assertNoSymlink(this.path, true);
      await rename(temporaryPath, this.path);
    } catch (error) {
      await handle?.close().catch(() => undefined);
      await unlink(temporaryPath).catch(() => undefined);
      if (error instanceof AuthNodeError) throw error;
      throw cacheIoError(error, 'cache_write_failed');
    }
  }

  async #acquireLock(signal?: AbortSignal): Promise<FileHandle> {
    const directory = dirname(this.path);
    await preparePrivateDirectory(directory, this.#platform);
    const lockPath = `${this.path}.lock`;
    const deadline = this.#now() + this.#lockTimeoutMs;
    for (;;) {
      throwIfAborted(signal);
      await assertNoSymlink(lockPath, true);
      try {
        return await open(lockPath, 'wx', PRIVATE_FILE_MODE);
      } catch (error) {
        if (!nodeError(error, 'EEXIST')) throw cacheIoError(error, 'cache_write_failed');
      }
      if (this.#now() >= deadline)
        throw new AuthNodeError('cache_lock_timeout', { retryable: true });
      await this.#sleep(this.#lockRetryMs, signal);
    }
  }

  async #releaseLock(handle: FileHandle): Promise<void> {
    const lockPath = `${this.path}.lock`;
    let stillOwned = false;
    try {
      const held = await handle.stat();
      const current = await lstat(lockPath);
      stillOwned = held.dev === current.dev && held.ino === current.ino;
    } catch {
      stillOwned = false;
    }
    await handle.close().catch(() => undefined);
    if (stillOwned) await unlink(lockPath).catch(() => undefined);
  }
}

function parseDocument(contents: string, namespace: string): CacheDocument {
  let value: unknown;
  try {
    value = JSON.parse(contents) as unknown;
  } catch {
    throw new AuthNodeError('cache_corrupt');
  }
  if (!isRecord(value) || value['version'] !== CACHE_VERSION || value['namespace'] !== namespace) {
    throw new AuthNodeError('cache_corrupt');
  }
  const rawProfiles = value['profiles'];
  if (!isRecord(rawProfiles)) throw new AuthNodeError('cache_corrupt');
  const profiles: Record<string, CachedTokenProfile> = {};
  for (const [name, profile] of Object.entries(rawProfiles)) {
    validateNameOrCorrupt(name);
    validateProfile(profile);
    profiles[name] = profile;
  }
  return { version: CACHE_VERSION, namespace, profiles };
}

function validateProfile(value: unknown): asserts value is CachedTokenProfile {
  if (
    !isRecord(value) ||
    !nonEmptyString(value['accessToken']) ||
    (value['refreshToken'] !== undefined && !nonEmptyString(value['refreshToken'])) ||
    (value['idToken'] !== undefined && !nonEmptyString(value['idToken'])) ||
    !nonEmptyString(value['tokenType']) ||
    typeof value['expiresAt'] !== 'number' ||
    !Number.isFinite(value['expiresAt']) ||
    value['expiresAt'] <= 0 ||
    !nonEmptyString(value['issuer']) ||
    !nonEmptyString(value['clientId']) ||
    !Array.isArray(value['scopes']) ||
    !value['scopes'].every(nonEmptyString) ||
    (value['audience'] !== undefined && !nonEmptyString(value['audience']))
  ) {
    throw new AuthNodeError('cache_corrupt');
  }
}

async function preparePrivateDirectory(path: string, platform: NodeJS.Platform): Promise<void> {
  try {
    await assertNoSymlink(path, false);
    await mkdir(path, { recursive: true, mode: PRIVATE_DIRECTORY_MODE });
    await assertNoSymlink(path, false);
    const metadata = await lstat(path);
    if (!metadata.isDirectory()) throw new AuthNodeError('cache_permission');
    if (platform !== 'win32' && (metadata.mode & UNSAFE_MODE_BITS) !== 0) {
      throw new AuthNodeError('cache_permission');
    }
  } catch (error) {
    if (error instanceof AuthNodeError) throw error;
    throw cacheIoError(error, 'cache_permission');
  }
}

async function assertNoSymlink(path: string, allowMissingLeaf: boolean): Promise<void> {
  const absolute = resolve(path);
  const root = parse(absolute).root;
  const parts = absolute.slice(root.length).split(sep).filter(Boolean);
  let current = root;
  for (let index = 0; index < parts.length; index += 1) {
    current = join(current, parts[index] ?? '');
    try {
      const metadata = await lstat(current);
      if (metadata.isSymbolicLink()) throw new AuthNodeError('cache_symlink');
    } catch (error) {
      if (nodeError(error, 'ENOENT') && (allowMissingLeaf || index < parts.length)) return;
      throw error;
    }
  }
}

function validateCacheFile(metadata: Stats, platform: NodeJS.Platform): void {
  if (!metadata.isFile()) throw new AuthNodeError('cache_permission');
  if (platform !== 'win32' && (metadata.mode & UNSAFE_MODE_BITS) !== 0) {
    throw new AuthNodeError('cache_permission');
  }
}

function validateName(value: string, label: string): void {
  if (!SAFE_NAME.test(value)) throw new TypeError(`${label} has an invalid format.`);
}

function validateNameOrCorrupt(value: string): void {
  if (!SAFE_NAME.test(value)) throw new AuthNodeError('cache_corrupt');
}

function positiveInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value <= 0)
    throw new RangeError(`${label} must be positive.`);
  return value;
}

function cacheIoError(
  error: unknown,
  fallback: 'cache_read_failed' | 'cache_write_failed' | 'cache_permission',
): AuthNodeError {
  return new AuthNodeError(
    nodeError(error, 'EACCES') || nodeError(error, 'EPERM') ? 'cache_permission' : fallback,
  );
}

function nodeError(error: unknown, code: string): boolean {
  return (
    error instanceof Error && 'code' in error && (error as NodeJS.ErrnoException).code === code
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

function ownProfile(
  profiles: Readonly<Record<string, CachedTokenProfile>>,
  name: string,
): CachedTokenProfile | undefined {
  return Object.hasOwn(profiles, name) ? profiles[name] : undefined;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted === true) throw new AuthNodeError('aborted');
}

function abortableDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolveDelay, reject) => {
    if (signal?.aborted === true) {
      reject(new AuthNodeError('aborted'));
      return;
    }
    const onAbort = (): void => {
      clearTimeout(timeout);
      reject(new AuthNodeError('aborted'));
    };
    const timeout = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolveDelay();
    }, milliseconds);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

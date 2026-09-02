import {
  SyncAuthError,
  SyncLocalApplyError,
  SyncNetworkError,
  SyncPartitionMismatchError,
  SyncPayloadCompatibilityError,
  SyncRateLimitError,
  SyncServerError,
  type SyncTransportError,
} from './error.js';

/** Minimal `fetch` shape the transport depends on. */
export interface SyncRequestInit {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  signal?: AbortSignal;
}

export type SyncFetch = (input: string, init?: SyncRequestInit) => Promise<SyncFetchResponse>;

/** A product API client's decoded request function. */
export type SyncPrebuiltRequest = <T>(input: string, init?: SyncRequestInit) => Promise<T>;

export interface SyncResponseHeaders {
  get(name: string): string | null;
}

/** Minimal `Response` shape the transport depends on. */
export interface SyncFetchResponse {
  readonly ok: boolean;
  readonly status: number;
  readonly headers?: SyncResponseHeaders;
  text(): Promise<string>;
}

export interface SyncFetchTransportOptions {
  /** Absolute base URL of the sync API, without a trailing slash. */
  baseUrl: string;
  fetch: SyncFetch;
  /** Resolves the headers that authenticate a request, per attempt. */
  authHeaders: () => Promise<Record<string, string>> | Record<string, string>;
  /**
   * Header carrying the local-data partition the caller expects to talk to.
   * Defaults to `X-Partition-Id`.
   */
  partitionHeader?: string;
  /**
   * Error code the server returns when the caller's partition no longer
   * exists. Defaults to `partition_identity_mismatch`.
   */
  partitionMismatchCode?: string;
  /** Milliseconds used when `Retry-After` is missing or unusable. Defaults to 60 seconds. */
  retryAfterFallbackMs?: number;
  /** Clock used to resolve `Retry-After`. Defaults to `Date.now`. */
  now?: () => number;
}

export interface SyncPrebuiltRequestTransportOptions {
  /**
   * Sends and decodes a request through a product API client. The function owns
   * base-URL resolution, authentication recovery, and failure classification.
   */
  request: SyncPrebuiltRequest;
  /** Defaults to `X-Partition-Id`. */
  partitionHeader?: string;
}

export type SyncTransportOptions = SyncFetchTransportOptions | SyncPrebuiltRequestTransportOptions;

export interface SyncRequestOptions {
  method?: string;
  query?: Record<string, string>;
  body?: unknown;
  /** Partition the caller expects; sent as the partition header when present. */
  partitionId?: string | null;
  signal?: AbortSignal;
}

const DEFAULT_PARTITION_HEADER = 'X-Partition-Id';
const DEFAULT_PARTITION_MISMATCH_CODE = 'partition_identity_mismatch';
const RETRYABLE_STATUSES = new Set([408, 429]);
export const DEFAULT_RETRY_AFTER_FALLBACK_MS = 60_000;

interface ServerErrorBody {
  code?: unknown;
  message?: unknown;
}

function isRetryableStatus(status: number): boolean {
  return RETRYABLE_STATUSES.has(status) || status >= 500;
}

export interface ParseRetryAfterOptions {
  /** Unix time in milliseconds. Defaults to `Date.now()`. */
  now?: number;
  /** Milliseconds added to `now` for missing, invalid, negative, or past values. */
  fallbackMs?: number;
}

/** Resolves an HTTP `Retry-After` delta or date to an ISO timestamp. */
export function parseRetryAfter(
  value: string | null | undefined,
  options: ParseRetryAfterOptions = {},
): string {
  const now = options.now ?? Date.now();
  const fallbackMs = options.fallbackMs ?? DEFAULT_RETRY_AFTER_FALLBACK_MS;
  if (!Number.isFinite(now)) throw new RangeError('now must be finite');
  if (!Number.isFinite(fallbackMs) || fallbackMs < 0) {
    throw new RangeError('retryAfterFallbackMs must be finite and non-negative');
  }

  const fallbackAt = now + fallbackMs;
  const trimmed = value?.trim() ?? '';
  let candidate = Number.NaN;
  if (/^\d+$/.test(trimmed)) {
    candidate = now + Number(trimmed) * 1_000;
  } else if (trimmed.length > 0) {
    candidate = Date.parse(trimmed);
  }
  return toIsoTimestamp(
    isSupportedTimestamp(candidate) && candidate >= now ? candidate : fallbackAt,
  );
}

function isSupportedTimestamp(milliseconds: number): boolean {
  return Number.isFinite(milliseconds) && !Number.isNaN(new Date(milliseconds).getTime());
}

function toIsoTimestamp(milliseconds: number): string {
  try {
    return new Date(milliseconds).toISOString();
  } catch {
    throw new RangeError('resolved Retry-After time is outside the supported date range');
  }
}

export type SyncCursorComparator<TCursor> = (left: TCursor, right: TCursor) => number;

export interface PullPagePosition<TCursor> {
  nextCursor: TCursor;
  hasMore: boolean;
}

/** Checks cursor monotonicity and pagination progress, then returns the page. */
export function validatePullPage<TCursor, TPage extends PullPagePosition<TCursor>>(
  currentCursor: TCursor,
  page: TPage,
  compare: SyncCursorComparator<TCursor>,
): TPage {
  const order = compare(page.nextCursor, currentCursor);
  if (!Number.isFinite(order)) {
    throw new SyncPayloadCompatibilityError('The pull cursor comparison was not finite.');
  }
  if (order < 0) {
    throw new SyncPayloadCompatibilityError('The pull cursor moved backwards.');
  }
  if (page.hasMore && order === 0) {
    throw new SyncPayloadCompatibilityError('The pull page did not advance its cursor.');
  }
  return page;
}

export interface CursorCommitOptions<TCursor, TResult> {
  nextCursor: TCursor;
  transaction: () => Promise<TResult>;
  commitCursor: (cursor: TCursor) => Promise<void> | void;
}

/** Commits a pull cursor only after the local transaction succeeds. */
export async function commitCursorAfterLocalTransaction<TCursor, TResult>({
  nextCursor,
  transaction,
  commitCursor,
}: CursorCommitOptions<TCursor, TResult>): Promise<TResult> {
  let result: TResult;
  try {
    result = await transaction();
  } catch (error) {
    if (error instanceof SyncLocalApplyError) throw error;
    throw new SyncLocalApplyError('The local pull transaction failed.', error);
  }
  try {
    await commitCursor(nextCursor);
  } catch (error) {
    if (error instanceof SyncLocalApplyError) throw error;
    throw new SyncLocalApplyError('The pull cursor could not be committed.', error);
  }
  return result;
}

function parseErrorBody(body: string): ServerErrorBody {
  try {
    const decoded: ServerErrorBody | null = JSON.parse(body) as ServerErrorBody | null;
    return typeof decoded === 'object' && decoded !== null ? decoded : {};
  } catch {
    return {};
  }
}

/**
 * Sync request plumbing for either a raw fetch or a product API client.
 *
 * Endpoint paths, request bodies, and response shapes stay product-defined:
 * callers name the path and the response type on every call.
 */
export class SyncTransport {
  private readonly baseUrl: string | null;
  private readonly partitionHeader: string;
  private readonly partitionMismatchCode: string | null;
  private readonly retryAfterFallbackMs: number;
  private readonly now: () => number;

  constructor(private readonly options: SyncTransportOptions) {
    this.baseUrl = 'fetch' in options ? options.baseUrl.replace(/\/+$/, '') : null;
    this.partitionHeader = options.partitionHeader ?? DEFAULT_PARTITION_HEADER;
    this.partitionMismatchCode =
      'fetch' in options
        ? (options.partitionMismatchCode ?? DEFAULT_PARTITION_MISMATCH_CODE)
        : null;
    this.retryAfterFallbackMs =
      'fetch' in options
        ? (options.retryAfterFallbackMs ?? DEFAULT_RETRY_AFTER_FALLBACK_MS)
        : DEFAULT_RETRY_AFTER_FALLBACK_MS;
    this.now = 'fetch' in options ? (options.now ?? Date.now) : Date.now;
    if (!Number.isFinite(this.retryAfterFallbackMs) || this.retryAfterFallbackMs < 0) {
      throw new RangeError('retryAfterFallbackMs must be finite and non-negative');
    }
  }

  /**
   * Sends one sync request and decodes its JSON response.
   *
   * The fetch-backed variant decodes JSON and maps failures to
   * {@link SyncTransportError}. The prebuilt-request variant delegates response
   * decoding and failure policy to that request function.
   */
  async request<T>(path: string, options: SyncRequestOptions = {}): Promise<T> {
    if ('request' in this.options) {
      return this.options.request<T>(this.path(path, options.query), this.init(options, {}));
    }
    const response = await this.send(path, options);
    const body = await this.readBody(response);
    if (!response.ok) {
      throw this.asTransportError(response.status, body, response.headers?.get('Retry-After'));
    }
    return this.decode(body) as T;
  }

  private async send(path: string, options: SyncRequestOptions): Promise<SyncFetchResponse> {
    if (!('fetch' in this.options)) {
      throw new Error('unreachable request-backed transport');
    }
    const headers: Record<string, string> = { ...(await this.options.authHeaders()) };
    const init = this.init(options, headers);
    try {
      return await this.options.fetch(this.url(path, options.query), init);
    } catch (error) {
      throw new SyncNetworkError(errorMessage(error), error);
    }
  }

  private init(
    options: SyncRequestOptions,
    initialHeaders: Record<string, string>,
  ): SyncRequestInit {
    const headers = { ...initialHeaders };
    if (options.body !== undefined) {
      headers['Content-Type'] = 'application/json';
    }
    if (options.partitionId != null) {
      headers[this.partitionHeader] = options.partitionId;
    }
    const init: SyncRequestInit = { headers };
    if (options.method !== undefined) {
      init.method = options.method;
    }
    if (options.body !== undefined) {
      init.body = JSON.stringify(options.body);
    }
    if (options.signal !== undefined) {
      init.signal = options.signal;
    }
    return init;
  }

  private async readBody(response: SyncFetchResponse): Promise<string> {
    try {
      return await response.text();
    } catch (error) {
      throw new SyncNetworkError(errorMessage(error), error);
    }
  }

  private decode(body: string): unknown {
    if (body.length === 0) {
      return undefined;
    }
    try {
      return JSON.parse(body);
    } catch (error) {
      throw new SyncPayloadCompatibilityError('The sync response was not valid JSON.', error);
    }
  }

  private url(path: string, query: Record<string, string> | undefined): string {
    return `${this.baseUrl ?? ''}${this.path(path, query)}`;
  }

  private path(path: string, query: Record<string, string> | undefined): string {
    const suffix = query ? `?${new URLSearchParams(query).toString()}` : '';
    return `${path}${suffix}`;
  }

  private asTransportError(
    status: number,
    body: string,
    retryAfter: string | null | undefined,
  ): SyncTransportError {
    const decoded = parseErrorBody(body);
    const message =
      typeof decoded.message === 'string' && decoded.message.length > 0
        ? decoded.message
        : `sync request failed with status ${String(status)}`;
    if (this.partitionMismatchCode !== null && decoded.code === this.partitionMismatchCode) {
      return new SyncPartitionMismatchError(message);
    }
    if (status === 401) {
      return new SyncAuthError(message);
    }
    if (status === 429) {
      const retryAt = parseRetryAfter(retryAfter, {
        now: this.now(),
        fallbackMs: this.retryAfterFallbackMs,
      });
      return new SyncRateLimitError(message, retryAt, retryAfter ?? null);
    }
    return new SyncServerError(message, isRetryableStatus(status));
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

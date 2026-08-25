import { SyncAuthError, SyncPartitionMismatchError, SyncTransportError } from './error.js';

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

/** Minimal `Response` shape the transport depends on. */
export interface SyncFetchResponse {
  readonly ok: boolean;
  readonly status: number;
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

interface ServerErrorBody {
  code?: unknown;
  message?: unknown;
}

function isRetryableStatus(status: number): boolean {
  return RETRYABLE_STATUSES.has(status) || status >= 500;
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

  constructor(private readonly options: SyncTransportOptions) {
    this.baseUrl = 'fetch' in options ? options.baseUrl.replace(/\/+$/, '') : null;
    this.partitionHeader = options.partitionHeader ?? DEFAULT_PARTITION_HEADER;
    this.partitionMismatchCode =
      'fetch' in options
        ? (options.partitionMismatchCode ?? DEFAULT_PARTITION_MISMATCH_CODE)
        : null;
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
      throw this.asTransportError(response.status, body);
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
      throw new SyncTransportError(errorMessage(error), true, error);
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
      throw new SyncTransportError(errorMessage(error), true, error);
    }
  }

  private decode(body: string): unknown {
    if (body.length === 0) {
      return undefined;
    }
    try {
      return JSON.parse(body);
    } catch (error) {
      throw new SyncTransportError('sync response was not valid JSON', false, error);
    }
  }

  private url(path: string, query: Record<string, string> | undefined): string {
    return `${this.baseUrl ?? ''}${this.path(path, query)}`;
  }

  private path(path: string, query: Record<string, string> | undefined): string {
    const suffix = query ? `?${new URLSearchParams(query).toString()}` : '';
    return `${path}${suffix}`;
  }

  private asTransportError(status: number, body: string): SyncTransportError {
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
    return new SyncTransportError(message, isRetryableStatus(status));
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

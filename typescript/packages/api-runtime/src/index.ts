/// <reference lib="dom" />

import createClient from 'openapi-fetch';
import type { Client, ClientOptions } from 'openapi-fetch';

/** The request ID header accepted and returned by `baukit-http`. */
export const REQUEST_ID_HEADER = 'x-request-id' as const;

/** An explicitly selected API environment. */
export interface ApiEnvironmentConfig {
  readonly baseUrl: string;
  readonly environment: string;
}

/** A product-owned map from environment names to API base URLs. */
export type ApiEnvironmentMap = Readonly<Record<string, string>>;

/** Resolves an explicit environment name from a caller-provided map. */
export function resolveApiEnvironment(
  environment: string,
  environments: ApiEnvironmentMap,
): ApiEnvironmentConfig {
  const baseUrl = Object.hasOwn(environments, environment) ? environments[environment] : undefined;
  if (baseUrl === undefined) {
    throw new Error(`Unknown API environment: ${environment}`);
  }

  return { baseUrl: normalizeBaseUrl(baseUrl), environment };
}

/** Supplies an access token for each logical request. Token lifecycle and caching stay product-owned. */
export type TokenProvider = () => Promise<string | null>;

/** Supplies a W3C `traceparent` value. No tracing provider is installed by this package. */
export type TraceparentProvider = (request: Request) => string | null | Promise<string | null>;

/** HTTP methods that may be opted into automatic retries. */
export type RetryableMethod = 'GET' | 'HEAD' | 'OPTIONS' | 'PUT' | 'DELETE';

/** Retry behavior for transient failures. */
export interface RetryOptions {
  /** Additional attempts after the initial attempt. Defaults to 2. */
  readonly maxRetries?: number;
  /** Initial backoff ceiling in milliseconds. Defaults to 100. */
  readonly baseDelayMs?: number;
  /** Maximum backoff ceiling in milliseconds. Defaults to 2,000. */
  readonly maxDelayMs?: number;
  /** Methods eligible for retries. Defaults to GET and HEAD. */
  readonly methods?: readonly RetryableMethod[];
  /** Random source used for full jitter. Defaults to `Math.random`. */
  readonly random?: () => number;
  /** Abort-aware delay seam, primarily useful for deterministic tests. */
  readonly sleep?: (delayMs: number, signal: AbortSignal | null) => Promise<void>;
}

/** The fetch shape accepted by browsers, React Native, and `openapi-fetch`. */
export type FetchImplementation = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

/** Context delivered when a response has status 401. */
export interface UnauthorizedContext {
  readonly error: ApiError | HttpError;
  readonly request: Request;
  readonly response: Response;
  /** False when the request body cannot be cloned safely for replay. */
  readonly canRetry: boolean;
}

/** Explicit outcome of a 401 hook. `handled` preserves the original normalized rejection. */
export type UnauthorizedRecovery = 'handled' | 'retry-once';

/** Backward-compatible 401 notification or explicit recovery callback. */
export type UnauthorizedHandler =
  | ((context: UnauthorizedContext) => void)
  | ((context: UnauthorizedContext) => Promise<void>)
  | ((context: UnauthorizedContext) => UnauthorizedRecovery)
  | ((context: UnauthorizedContext) => Promise<UnauthorizedRecovery>);

/** Options shared by the standalone fetch wrapper and generated clients. */
export interface ApiRuntimeOptions extends ApiEnvironmentConfig {
  readonly fetch?: FetchImplementation;
  readonly requestConstructor?: typeof Request;
  readonly tokenProvider?: TokenProvider;
  readonly traceparentProvider?: TraceparentProvider;
  readonly requestIdFactory?: () => string;
  readonly onUnauthorized?: UnauthorizedHandler;
  readonly retry?: RetryOptions | false;
}

/** A configured runtime that can be handed to a product's generated client. */
export interface ApiRuntime extends ApiEnvironmentConfig {
  readonly fetch: FetchImplementation;
}

/** JSON values accepted in Baukit error details. */
export type JsonValue =
  null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

/** A validated Baukit error envelope. */
export interface ApiErrorEnvelope<Code extends string = string> {
  readonly error: {
    readonly code: Code;
    readonly message: string;
    readonly request_id: string;
    readonly details: Readonly<Record<string, JsonValue>>;
  };
}

/** A non-success response using Baukit's standard JSON error envelope. */
export class ApiError<Code extends string = string> extends Error {
  readonly kind = 'api' as const;
  readonly code: Code;
  readonly requestId: string;
  readonly details: Readonly<Record<string, JsonValue>>;
  readonly status: number;

  constructor(envelope: ApiErrorEnvelope<Code>, status: number) {
    super(envelope.error.message);
    this.name = 'ApiError';
    this.code = envelope.error.code;
    this.requestId = envelope.error.request_id;
    this.details = envelope.error.details;
    this.status = status;
  }
}

/** A transport failure where no HTTP response was available (including CORS failures). */
export class NetworkError extends Error {
  readonly kind = 'network' as const;
  readonly requestId: string;
  readonly aborted: boolean;

  constructor(message: string, requestId: string, cause: unknown, aborted = false) {
    super(message, { cause });
    this.name = 'NetworkError';
    this.requestId = requestId;
    this.aborted = aborted;
  }
}

/** A non-success HTTP response that did not contain a valid Baukit error envelope. */
export class HttpError extends Error {
  readonly kind = 'http' as const;
  readonly status: number;
  readonly requestId: string | null;
  readonly body: unknown;

  constructor(response: Response, body: unknown) {
    const suffix = response.statusText === '' ? '' : ` ${response.statusText}`;
    super(`HTTP ${String(response.status)}${suffix}`);
    this.name = 'HttpError';
    this.status = response.status;
    this.requestId = response.headers.get(REQUEST_ID_HEADER);
    this.body = body;
  }
}

/** Narrows an unknown value to an ApiError, optionally with one specific code. */
export function isApiError(value: unknown): value is ApiError;
export function isApiError<Code extends string>(
  value: unknown,
  code: Code,
): value is ApiError<Code>;
export function isApiError(value: unknown, code?: string): value is ApiError {
  return value instanceof ApiError && (code === undefined || value.code === code);
}

/** Narrows an unknown value to a NetworkError. */
export function isNetworkError(value: unknown): value is NetworkError {
  return value instanceof NetworkError;
}

/** Narrows an unknown value to an HttpError. */
export function isHttpError(value: unknown): value is HttpError {
  return value instanceof HttpError;
}

/** Parses the exact Baukit error envelope shape, returning null for any other value. */
export function parseApiErrorEnvelope(value: unknown, status: number): ApiError | null {
  if (!isRecord(value) || !isRecord(value['error'])) {
    return null;
  }

  const error = value['error'];
  if (
    typeof error['code'] !== 'string' ||
    typeof error['message'] !== 'string' ||
    typeof error['request_id'] !== 'string' ||
    !isJsonObject(error['details'])
  ) {
    return null;
  }

  return new ApiError(
    {
      error: {
        code: error['code'],
        message: error['message'],
        request_id: error['request_id'],
        details: error['details'],
      },
    },
    status,
  );
}

/** Converts a non-success HTTP response into an ApiError or HttpError. */
export async function normalizeResponseError(response: Response): Promise<ApiError | HttpError> {
  let body: unknown;
  try {
    const text = await response.clone().text();
    if (text !== '') {
      try {
        body = JSON.parse(text) as unknown;
      } catch {
        body = text;
      }
    }
  } catch {
    body = undefined;
  }

  return parseApiErrorEnvelope(body, response.status) ?? new HttpError(response, body);
}

/** Creates the fetch runtime used by standalone callers and generated clients. */
export function createApiRuntime(options: ApiRuntimeOptions): ApiRuntime {
  const baseUrl = normalizeBaseUrl(options.baseUrl);
  return {
    baseUrl,
    environment: options.environment,
    fetch: createApiFetch({ ...options, baseUrl }),
  };
}

/** Creates an `openapi-fetch` client backed by the Baukit runtime. */
export function createApiClient<Paths extends object>(
  runtimeOptions: ApiRuntimeOptions,
  clientOptions: Omit<ClientOptions, 'baseUrl' | 'fetch'> = {},
): Client<Paths> {
  const runtime = createApiRuntime(runtimeOptions);
  return createClient<Paths>({ ...clientOptions, baseUrl: runtime.baseUrl, fetch: runtime.fetch });
}

/** Creates a standalone fetch wrapper with auth, request IDs, errors, and safe retries. */
export function createApiFetch(options: ApiRuntimeOptions): FetchImplementation {
  const baseUrl = normalizeBaseUrl(options.baseUrl);
  const requestConstructor = options.requestConstructor ?? globalThis.Request;
  const transport = options.fetch ?? globalThis.fetch.bind(globalThis);
  const requestIdFactory = options.requestIdFactory ?? createUuid;
  const retry = normalizeRetryOptions(options.retry);

  return async (input, init) => {
    const requestId = requestIdFactory();
    let request: Request;
    try {
      request = createRequest(input, init, baseUrl, requestConstructor);
      request = await decorateRequest(request, requestId, requestConstructor, options);
    } catch (cause) {
      if (cause instanceof NetworkError) {
        throw cause;
      }
      throw networkError(cause, requestId, init?.signal ?? null);
    }

    let replaySource = cloneRequest(request);
    let useOriginalRequest = true;
    let retries = 0;
    let unauthorizedReplayUsed = false;
    for (;;) {
      throwIfAborted(request.signal, requestId);
      try {
        const attempt = useOriginalRequest ? request : replaySource?.clone();
        if (attempt === undefined) {
          throw new TypeError('API request body cannot be replayed');
        }
        useOriginalRequest = false;
        const response = await transport(attempt);
        throwIfAborted(request.signal, requestId);
        if (
          replaySource !== undefined &&
          shouldRetryResponse(request.method, response.status, retries, retry)
        ) {
          await retryDelay(retries, retry, request.signal, requestId);
          retries += 1;
          continue;
        }

        if (response.ok) {
          return response;
        }

        const error = await normalizeResponseError(response);
        if (
          response.status === 401 &&
          options.onUnauthorized !== undefined &&
          !unauthorizedReplayUsed
        ) {
          let recovery: unknown;
          try {
            recovery = await options.onUnauthorized({
              error,
              request: replaySource?.clone() ?? request,
              response,
              canRetry: replaySource !== undefined,
            });
          } catch {
            // Re-authentication failures must not replace the normalized API failure.
          }
          throwIfAborted(request.signal, requestId);
          if (recovery === 'retry-once' && replaySource !== undefined) {
            unauthorizedReplayUsed = true;
            try {
              request = await decorateRequest(
                replaySource.clone(),
                requestId,
                requestConstructor,
                options,
              );
            } catch (cause) {
              throwIfAborted(request.signal, requestId);
              if (cause instanceof NetworkError) {
                throw cause;
              }
              throw error;
            }
            replaySource = cloneRequest(request);
            if (replaySource === undefined) {
              throw error;
            }
            useOriginalRequest = true;
            retries = 0;
            continue;
          }
        }
        throw error;
      } catch (cause) {
        if (cause instanceof ApiError || cause instanceof HttpError) {
          throw cause;
        }

        const aborted = request.signal.aborted || isAbortError(cause);
        if (
          !aborted &&
          replaySource !== undefined &&
          shouldRetryMethod(request.method, retry) &&
          retries < retry.maxRetries
        ) {
          await retryDelay(retries, retry, request.signal, requestId);
          retries += 1;
          continue;
        }
        throw new NetworkError(
          aborted ? 'API request was aborted' : 'API network request failed',
          requestId,
          cause,
          aborted,
        );
      }
    }
  };
}

async function decorateRequest(
  request: Request,
  requestId: string,
  requestConstructor: typeof Request,
  options: ApiRuntimeOptions,
): Promise<Request> {
  throwIfAborted(request.signal, requestId);
  const headers = new Headers(request.headers);
  headers.set(REQUEST_ID_HEADER, requestId);

  if (options.tokenProvider !== undefined) {
    const token = await options.tokenProvider();
    throwIfAborted(request.signal, requestId);
    headers.delete('authorization');
    if (token !== null) {
      headers.set('authorization', `Bearer ${token}`);
    }
  }

  if (options.traceparentProvider !== undefined) {
    const traceparent = await options.traceparentProvider(request);
    throwIfAborted(request.signal, requestId);
    if (traceparent !== null) {
      headers.set('traceparent', traceparent);
    }
  }

  return new requestConstructor(request, { headers });
}

function cloneRequest(request: Request): Request | undefined {
  try {
    return request.clone();
  } catch {
    return undefined;
  }
}

/** Expected request fields used by MockFetch assertions. */
export interface MockRequestExpectation {
  readonly method?: string;
  readonly url?: string;
  readonly headers?: Readonly<Record<string, string>>;
}

/** A queued response factory for MockFetch. */
export type MockFetchHandler = (request: Request) => Response | Promise<Response>;

/** A small queued fetch transport for product and package tests. */
export class MockFetch {
  readonly requests: Request[] = [];
  readonly fetch: FetchImplementation;
  readonly #queue: (Response | Error | MockFetchHandler)[] = [];

  constructor() {
    this.fetch = async (input, init) => {
      const request = new Request(input, init);
      this.requests.push(request.clone());
      const next = this.#queue.shift();
      if (next === undefined) {
        throw new Error('MockFetch has no queued response');
      }
      if (next instanceof Error) {
        throw next;
      }
      return typeof next === 'function' ? next(request) : next;
    };
  }

  /** Adds a response, transport error, or response factory to the queue. */
  enqueue(next: Response | Error | MockFetchHandler): this {
    this.#queue.push(next);
    return this;
  }

  /** Adds a JSON response to the queue. */
  enqueueJson(body: unknown, init: ResponseInit = {}): this {
    const headers = new Headers(init.headers);
    if (!headers.has('content-type')) {
      headers.set('content-type', 'application/json');
    }
    return this.enqueue(new Response(JSON.stringify(body), { ...init, headers }));
  }

  /** Returns one recorded request or throws a clear assertion error. */
  request(index: number): Request {
    const request = this.requests[index];
    if (request === undefined) {
      throw new Error(`MockFetch request ${String(index)} was not recorded`);
    }
    return request;
  }

  /** Asserts selected fields of a recorded request without a test-framework dependency. */
  assertRequest(index: number, expected: MockRequestExpectation): void {
    const request = this.request(index);
    if (expected.method !== undefined && request.method !== expected.method.toUpperCase()) {
      throw new Error(`Expected request method ${expected.method}, received ${request.method}`);
    }
    if (expected.url !== undefined && request.url !== expected.url) {
      throw new Error(`Expected request URL ${expected.url}, received ${request.url}`);
    }
    for (const [name, value] of Object.entries(expected.headers ?? {})) {
      const received = request.headers.get(name);
      if (received !== value) {
        throw new Error(`Expected request header ${name}=${value}, received ${String(received)}`);
      }
    }
  }

  /** Asserts that every queued result has been consumed. */
  assertQueueEmpty(): void {
    if (this.#queue.length !== 0) {
      throw new Error(`MockFetch has ${String(this.#queue.length)} queued result(s) remaining`);
    }
  }
}

interface NormalizedRetryOptions {
  readonly maxRetries: number;
  readonly baseDelayMs: number;
  readonly maxDelayMs: number;
  readonly methods: ReadonlySet<RetryableMethod>;
  readonly random: () => number;
  readonly sleep: (delayMs: number, signal: AbortSignal | null) => Promise<void>;
}

const NEVER_RETRY = new Set(['POST', 'PATCH']);
const RETRYABLE_METHODS = new Set<RetryableMethod>(['GET', 'HEAD', 'OPTIONS', 'PUT', 'DELETE']);
const RETRYABLE_STATUSES = new Set([502, 503, 504]);

function normalizeBaseUrl(baseUrl: string): string {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch {
    throw new Error(`API base URL must be absolute: ${baseUrl}`);
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`API base URL must use HTTP or HTTPS: ${baseUrl}`);
  }
  if (parsed.search !== '' || parsed.hash !== '') {
    throw new Error(`API base URL must not contain a query or fragment: ${baseUrl}`);
  }
  return parsed.toString().replace(/\/$/u, '');
}

function createRequest(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  baseUrl: string,
  requestConstructor: typeof Request,
): Request {
  if (typeof input === 'string' || input instanceof URL) {
    return new requestConstructor(new URL(input.toString(), `${baseUrl}/`), init);
  }
  return new requestConstructor(input, init);
}

function createUuid(): string {
  const cryptoApi = (globalThis as unknown as { readonly crypto?: Crypto }).crypto;
  if (typeof cryptoApi?.randomUUID === 'function') {
    return cryptoApi.randomUUID();
  }

  const bytes = new Uint8Array(16);
  if (typeof cryptoApi?.getRandomValues === 'function') {
    cryptoApi.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function normalizeRetryOptions(options: RetryOptions | false | undefined): NormalizedRetryOptions {
  const maxRetries = options === false ? 0 : (options?.maxRetries ?? 2);
  const baseDelayMs = options === false ? 100 : (options?.baseDelayMs ?? 100);
  const maxDelayMs = options === false ? 2_000 : (options?.maxDelayMs ?? 2_000);
  assertFiniteNonNegativeInteger(maxRetries, 'maxRetries');
  assertFiniteNonNegative(baseDelayMs, 'baseDelayMs');
  assertFiniteNonNegative(maxDelayMs, 'maxDelayMs');
  const requestedMethods = options === false ? [] : (options?.methods ?? ['GET', 'HEAD']);
  const methods = new Set(
    requestedMethods.filter(
      (method): method is RetryableMethod =>
        RETRYABLE_METHODS.has(method) && !NEVER_RETRY.has(method),
    ),
  );
  return {
    maxRetries,
    baseDelayMs,
    maxDelayMs,
    methods,
    random: options === false ? Math.random : (options?.random ?? Math.random),
    sleep: options === false ? abortableSleep : (options?.sleep ?? abortableSleep),
  };
}

function assertFiniteNonNegative(value: number, name: string): void {
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`${name} must be a finite non-negative number`);
  }
}

function assertFiniteNonNegativeInteger(value: number, name: string): void {
  assertFiniteNonNegative(value, name);
  if (!Number.isInteger(value)) {
    throw new Error(`${name} must be an integer`);
  }
}

function shouldRetryMethod(method: string, retry: NormalizedRetryOptions): boolean {
  return !NEVER_RETRY.has(method) && retry.methods.has(method as RetryableMethod);
}

function shouldRetryResponse(
  method: string,
  status: number,
  retries: number,
  retry: NormalizedRetryOptions,
): boolean {
  return (
    retries < retry.maxRetries && shouldRetryMethod(method, retry) && RETRYABLE_STATUSES.has(status)
  );
}

async function retryDelay(
  retryIndex: number,
  retry: NormalizedRetryOptions,
  signal: AbortSignal,
  requestId: string,
): Promise<void> {
  const ceiling = Math.min(retry.maxDelayMs, retry.baseDelayMs * 2 ** retryIndex);
  const random = Math.max(0, Math.min(1, retry.random()));
  try {
    await retry.sleep(ceiling * random, signal);
  } catch (cause) {
    throw new NetworkError('API request was aborted', requestId, cause, true);
  }
  throwIfAborted(signal, requestId);
}

function abortableSleep(delayMs: number, signal: AbortSignal | null): Promise<void> {
  if (signal === null) {
    return new Promise((resolve) => {
      globalThis.setTimeout(resolve, delayMs);
    });
  }
  if (signal.aborted) {
    return Promise.reject(abortReason(signal));
  }
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      globalThis.clearTimeout(timer);
      reject(abortReason(signal));
    };
    const timer = globalThis.setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, delayMs);
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error
    ? signal.reason
    : new Error('API request was aborted', { cause: signal.reason });
}

function throwIfAborted(signal: AbortSignal, requestId: string): void {
  if (signal.aborted) {
    throw new NetworkError('API request was aborted', requestId, signal.reason, true);
  }
}

function networkError(cause: unknown, requestId: string, signal: AbortSignal | null): NetworkError {
  const aborted = signal?.aborted === true || isAbortError(cause);
  return new NetworkError(
    aborted ? 'API request was aborted' : 'API network request failed',
    requestId,
    cause,
    aborted,
  );
}

function isAbortError(value: unknown): boolean {
  return value instanceof Error && value.name === 'AbortError';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isJsonObject(value: unknown): value is Readonly<Record<string, JsonValue>> {
  return isRecord(value) && Object.values(value).every(isJsonValue);
}

function isJsonValue(value: unknown): value is JsonValue {
  if (
    value === null ||
    typeof value === 'string' ||
    typeof value === 'boolean' ||
    (typeof value === 'number' && Number.isFinite(value))
  ) {
    return true;
  }
  if (Array.isArray(value)) {
    return value.every(isJsonValue);
  }
  return isJsonObject(value);
}

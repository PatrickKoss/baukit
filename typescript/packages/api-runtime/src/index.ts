/// <reference lib="dom" />

import createClient from 'openapi-fetch';
import type { Client, ClientOptions } from 'openapi-fetch';

import type { ApiError, HttpError } from './errors.js';
import { fetchSettings, runFetch } from './fetch.js';

export {
  unverifiedDisplayIdentityHintsFromJwt,
  type UnverifiedDisplayIdentityFallback,
  type UnverifiedDisplayIdentityHints,
} from './unverified-display-identity.js';
export {
  unverifiedSignInSessionFromJwt,
  type SignInOrganization,
  type SignInSession,
  type SignInUser,
} from './sign-in-session.js';

export { IDEMPOTENCY_KEY_HEADER, REQUEST_ID_HEADER } from './headers.js';
export {
  ApiError,
  HttpError,
  isApiError,
  isHttpError,
  isNetworkError,
  NetworkError,
  normalizeResponseError,
  parseApiErrorEnvelope,
  type ApiErrorEnvelope,
  type JsonValue,
} from './errors.js';

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

/**
 * HTTP methods that may be opted into automatic retries.
 *
 * `POST` and `PATCH` retry only when the request carries an `Idempotency-Key` header.
 */
export type RetryableMethod = 'GET' | 'HEAD' | 'OPTIONS' | 'PUT' | 'DELETE' | 'POST' | 'PATCH';

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

/** Notification emitted when an explicit credential replay also returns 401. */
export type UnauthorizedExhaustedHandler = (context: UnauthorizedContext) => void | Promise<void>;

/** Notification for a fenced product profile. Credential recovery is skipped. */
export type ProfileErasedHandler = (context: UnauthorizedContext) => void | Promise<void>;

/** Options shared by the standalone fetch wrapper and generated clients. */
export interface ApiRuntimeOptions extends ApiEnvironmentConfig {
  readonly fetch?: FetchImplementation;
  readonly requestConstructor?: typeof Request;
  readonly tokenProvider?: TokenProvider;
  readonly traceparentProvider?: TraceparentProvider;
  readonly requestIdFactory?: () => string;
  readonly onUnauthorized?: UnauthorizedHandler;
  readonly onUnauthorizedExhausted?: UnauthorizedExhaustedHandler;
  readonly onProfileErased?: ProfileErasedHandler;
  readonly retry?: RetryOptions | false;
}

/** A configured runtime that can be handed to a product's generated client. */
export interface ApiRuntime extends ApiEnvironmentConfig {
  readonly fetch: FetchImplementation;
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
  const settings = fetchSettings(options, normalizeBaseUrl(options.baseUrl));
  return (input, init) => runFetch(settings, input, init);
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

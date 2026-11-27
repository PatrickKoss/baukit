import { OriginCooldown } from './cooldown.js';
import {
  ApiError,
  ApiUnavailableError,
  HttpError,
  isApiError,
  NetworkError,
  normalizeResponseError,
} from './errors.js';
import { REQUEST_ID_HEADER } from './headers.js';
import type {
  ApiRuntimeOptions,
  FetchImplementation,
  UnauthorizedContext,
  UnauthorizedHandler,
  UnauthorizedRecovery,
} from './index.js';
import {
  isAbortError,
  networkError,
  nextRetryDelayMs,
  normalizeRetryOptions,
  retryAfterDelayMs,
  retryDelay,
  shouldRetryMethod,
  shouldRetryResponse,
  throwIfAborted,
  type NormalizedRetryOptions,
} from './retry.js';
import { attemptSignal } from './timeout.js';

const UNAUTHORIZED_STATUS = 401;
const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;
const RETRY = Symbol('retry');
type AttemptOutcome = Response | typeof RETRY;

export interface FetchSettings {
  readonly baseUrl: string;
  readonly requestConstructor: typeof Request;
  readonly transport: FetchImplementation;
  readonly requestIdFactory: () => string;
  readonly retry: NormalizedRetryOptions;
  readonly requestTimeoutMs: number | null;
  readonly cooldown: OriginCooldown | null;
  readonly now: () => number;
  readonly options: ApiRuntimeOptions;
}

interface FetchContext extends FetchSettings {
  readonly requestId: string;
  /** Session subject at send time; 401 signals from an older session are dropped. */
  readonly sessionSubject: string | undefined;
}

interface AttemptState {
  request: Request;
  replaySource: Request | undefined;
  useOriginalRequest: boolean;
  retries: number;
  unauthorizedReplayUsed: boolean;
  timedOut: boolean;
}

export function fetchSettings(options: ApiRuntimeOptions, baseUrl: string): FetchSettings {
  const now = options.now ?? Date.now;
  return {
    baseUrl,
    requestConstructor: options.requestConstructor ?? globalThis.Request,
    transport: options.fetch ?? globalThis.fetch.bind(globalThis),
    requestIdFactory: options.requestIdFactory ?? createUuid,
    retry: normalizeRetryOptions(options.retry),
    requestTimeoutMs: normalizeRequestTimeout(options.requestTimeoutMs),
    cooldown: options.cooldown === false ? null : new OriginCooldown(options.cooldown ?? {}, now),
    now,
    options,
  };
}

export async function runFetch(
  settings: FetchSettings,
  input: RequestInfo | URL,
  init: RequestInit | undefined,
): Promise<Response> {
  const context: FetchContext = {
    ...settings,
    requestId: settings.requestIdFactory(),
    sessionSubject: settings.options.sessionSubject?.(),
  };
  const request = await prepareRequest(context, input, init);
  const state: AttemptState = {
    request,
    replaySource: cloneRequest(request),
    useOriginalRequest: true,
    retries: 0,
    unauthorizedReplayUsed: false,
    timedOut: false,
  };
  for (;;) {
    throwIfAborted(state.request.signal, context.requestId);
    throwIfCoolingDown(context, state);
    const outcome = await runAttempt(context, state);
    if (outcome !== RETRY) {
      return outcome;
    }
  }
}

async function prepareRequest(
  context: FetchContext,
  input: RequestInfo | URL,
  init: RequestInit | undefined,
): Promise<Request> {
  try {
    const request = createRequest(input, init, context.baseUrl, context.requestConstructor);
    return await decorateRequest(context, request);
  } catch (cause) {
    if (cause instanceof NetworkError) {
      throw cause;
    }
    throw networkError(cause, context.requestId, init?.signal ?? null);
  }
}

async function runAttempt(context: FetchContext, state: AttemptState): Promise<AttemptOutcome> {
  try {
    const response = await sendAttempt(context, state);
    return await handleResponse(context, state, response);
  } catch (cause) {
    return handleFailure(context, state, cause);
  }
}

function normalizeRequestTimeout(timeoutMs: number | false | undefined): number | null {
  if (timeoutMs === false) {
    return null;
  }
  const value = timeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error('requestTimeoutMs must be a finite positive number');
  }
  return value;
}

function originOf(state: AttemptState): string {
  return new URL(state.request.url).origin;
}

function throwIfCoolingDown(context: FetchContext, state: AttemptState): void {
  const retryAt = context.cooldown?.blockedUntil(originOf(state)) ?? null;
  if (retryAt !== null) {
    throw new ApiUnavailableError(context.requestId, retryAt);
  }
}

async function sendAttempt(context: FetchContext, state: AttemptState): Promise<Response> {
  const attempt = state.useOriginalRequest ? state.request : state.replaySource?.clone();
  if (attempt === undefined) {
    throw new TypeError('API request body cannot be replayed');
  }
  state.useOriginalRequest = false;
  state.timedOut = false;
  const scope = attemptSignal(state.request.signal, context.requestTimeoutMs);
  try {
    const response = await context.transport(attempt, { signal: scope.signal });
    throwIfAborted(state.request.signal, context.requestId);
    context.cooldown?.recordResponse(originOf(state));
    return response;
  } catch (cause) {
    state.timedOut = scope.timedOut();
    throw cause;
  } finally {
    scope.dispose();
  }
}

async function handleResponse(
  context: FetchContext,
  state: AttemptState,
  response: Response,
): Promise<AttemptOutcome> {
  const retryAfterMs = retryAfterDelayMs(response, context.retry, context.now());
  if (retryAfterMs !== null) {
    context.cooldown?.coolDown(originOf(state), retryAfterMs);
  }
  if (
    state.replaySource !== undefined &&
    shouldRetryResponse(
      state.request,
      { status: response.status, retryAfterMs },
      state.retries,
      context.retry,
    )
  ) {
    await backOff(context, state, retryAfterMs);
    return RETRY;
  }
  if (response.ok) {
    return response;
  }
  const error = await normalizeResponseError(response);
  if (response.status !== UNAUTHORIZED_STATUS || !acceptsSessionSignals(context, state.request)) {
    throw error;
  }
  return handleUnauthorized(context, state, { response, error });
}

interface UnauthorizedResponse {
  readonly response: Response;
  readonly error: ApiError | HttpError;
}

async function handleUnauthorized(
  context: FetchContext,
  state: AttemptState,
  failure: UnauthorizedResponse,
): Promise<AttemptOutcome> {
  const { options, requestId } = context;
  const { error } = failure;
  const unauthorizedContext = (canRetry: boolean): UnauthorizedContext => ({
    ...failure,
    request: state.replaySource?.clone() ?? state.request,
    canRetry,
    ...(context.sessionSubject === undefined ? {} : { subject: context.sessionSubject }),
  });
  if (isApiError(error, 'profile_erased')) {
    await notifyObserver(() => options.onProfileErased?.(unauthorizedContext(false)));
    throwIfAborted(state.request.signal, requestId);
    throw error;
  }
  if (state.unauthorizedReplayUsed) {
    if (options.onUnauthorizedExhausted !== undefined) {
      const onExhausted = options.onUnauthorizedExhausted;
      await notifyObserver(() => onExhausted(unauthorizedContext(false)));
      throwIfAborted(state.request.signal, requestId);
    }
    throw error;
  }
  if (options.onUnauthorized === undefined) {
    throw error;
  }
  const recovery = await askForRecovery(
    options.onUnauthorized,
    unauthorizedContext(state.replaySource !== undefined),
  );
  throwIfAborted(state.request.signal, requestId);
  if (recovery !== 'retry-once' || state.replaySource === undefined) {
    throw error;
  }
  await replayWithFreshCredentials(context, state, state.replaySource, error);
  return RETRY;
}

/** Observer failures must not replace the normalized API failure. */
async function notifyObserver(notify: () => void | Promise<void>): Promise<void> {
  try {
    await notify();
  } catch {
    // The caller rethrows the original normalized error.
  }
}

async function askForRecovery(
  onUnauthorized: UnauthorizedHandler,
  context: UnauthorizedContext,
): Promise<UnauthorizedRecovery | undefined> {
  try {
    const recovery: unknown = await onUnauthorized(context);
    return recovery === 'retry-once' || recovery === 'handled' ? recovery : undefined;
  } catch {
    return undefined;
  }
}

async function replayWithFreshCredentials(
  context: FetchContext,
  state: AttemptState,
  replaySource: Request,
  error: ApiError | HttpError,
): Promise<void> {
  state.unauthorizedReplayUsed = true;
  try {
    state.request = await decorateRequest(context, replaySource.clone());
  } catch (cause) {
    throwIfAborted(state.request.signal, context.requestId);
    if (cause instanceof NetworkError) {
      throw cause;
    }
    throw error;
  }
  state.replaySource = cloneRequest(state.request);
  if (state.replaySource === undefined) {
    throw error;
  }
  state.useOriginalRequest = true;
  state.retries = 0;
}

async function handleFailure(
  context: FetchContext,
  state: AttemptState,
  cause: unknown,
): Promise<AttemptOutcome> {
  if (
    cause instanceof ApiError ||
    cause instanceof HttpError ||
    cause instanceof ApiUnavailableError
  ) {
    throw cause;
  }
  const aborted = state.request.signal.aborted || (!state.timedOut && isAbortError(cause));
  if (!aborted) {
    context.cooldown?.recordNetworkFailure(originOf(state));
  }
  if (
    !aborted &&
    state.replaySource !== undefined &&
    shouldRetryMethod(state.request, context.retry) &&
    state.retries < context.retry.maxRetries
  ) {
    await backOff(context, state, null);
    return RETRY;
  }
  throw new NetworkError(
    failureMessage(aborted, state.timedOut),
    context.requestId,
    cause,
    aborted,
  );
}

function failureMessage(aborted: boolean, timedOut: boolean): string {
  if (aborted) {
    return 'API request was aborted';
  }
  return timedOut ? 'API request timed out' : 'API network request failed';
}

async function backOff(
  context: FetchContext,
  state: AttemptState,
  retryAfterMs: number | null,
): Promise<void> {
  const delayMs = nextRetryDelayMs(state.retries, context.retry, retryAfterMs);
  const retryAt = context.cooldown?.blockedUntil(originOf(state)) ?? null;
  if (retryAt !== null && retryAt > context.now() + delayMs) {
    throw new ApiUnavailableError(context.requestId, retryAt);
  }
  await retryDelay(delayMs, context.retry, state.request.signal, context.requestId);
  state.retries += 1;
}

async function decorateRequest(context: FetchContext, request: Request): Promise<Request> {
  const { options, requestId } = context;
  throwIfAborted(request.signal, requestId);
  const headers = new Headers(request.headers);
  headers.set(REQUEST_ID_HEADER, requestId);

  if (options.tokenProvider !== undefined && isApiRequest(request.url, context.baseUrl)) {
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

  return new context.requestConstructor(request, { headers });
}

function cloneRequest(request: Request): Request | undefined {
  try {
    return request.clone();
  } catch {
    return undefined;
  }
}

function createRequest(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  baseUrl: string,
  requestConstructor: typeof Request,
): Request {
  if (typeof input === 'string' || input instanceof URL) {
    return new requestConstructor(resolveApiUrl(input, baseUrl), init);
  }
  return new requestConstructor(input, init);
}

const BASE_RELATIVE_PATH = /^\/(?![/\\])/u;

/** Paths starting with a single `/` stay under the base path; `//host` and `/\host` do not. */
export function resolveApiUrl(input: string | URL, baseUrl: string): URL {
  if (input instanceof URL) return input;
  if (BASE_RELATIVE_PATH.test(input)) return new URL(`${baseUrl}${input}`);
  return new URL(input, `${baseUrl}/`);
}

/** True when `url` has the base origin and sits at or under the base path. */
export function isApiRequest(url: string, baseUrl: string): boolean {
  let target: URL;
  try {
    target = new URL(url);
  } catch {
    return false;
  }
  const base = new URL(baseUrl);
  if (target.origin !== base.origin) return false;
  const basePath = base.pathname.replace(/\/$/u, '');
  return (
    basePath === '' || target.pathname === basePath || target.pathname.startsWith(`${basePath}/`)
  );
}

function acceptsSessionSignals(context: FetchContext, request: Request): boolean {
  if (!isApiRequest(request.url, context.baseUrl)) return false;
  const currentSubject = context.options.sessionSubject;
  return currentSubject === undefined || currentSubject() === context.sessionSubject;
}

function createUuid(): string {
  // Runtime probe: React Native and older runtimes may lack `crypto` despite the DOM lib typing.
  const runtime: { readonly crypto?: Crypto } = globalThis;
  const cryptoApi = runtime.crypto;
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

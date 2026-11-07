import { ApiError, HttpError, isApiError, NetworkError, normalizeResponseError } from './errors.js';
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
  normalizeRetryOptions,
  retryDelay,
  shouldRetryMethod,
  shouldRetryResponse,
  throwIfAborted,
  type NormalizedRetryOptions,
} from './retry.js';

const UNAUTHORIZED_STATUS = 401;
const RETRY = Symbol('retry');
type AttemptOutcome = Response | typeof RETRY;

export interface FetchSettings {
  readonly baseUrl: string;
  readonly requestConstructor: typeof Request;
  readonly transport: FetchImplementation;
  readonly requestIdFactory: () => string;
  readonly retry: NormalizedRetryOptions;
  readonly options: ApiRuntimeOptions;
}

interface FetchContext extends FetchSettings {
  readonly requestId: string;
}

interface AttemptState {
  request: Request;
  replaySource: Request | undefined;
  useOriginalRequest: boolean;
  retries: number;
  unauthorizedReplayUsed: boolean;
}

export function fetchSettings(options: ApiRuntimeOptions, baseUrl: string): FetchSettings {
  return {
    baseUrl,
    requestConstructor: options.requestConstructor ?? globalThis.Request,
    transport: options.fetch ?? globalThis.fetch.bind(globalThis),
    requestIdFactory: options.requestIdFactory ?? createUuid,
    retry: normalizeRetryOptions(options.retry),
    options,
  };
}

export async function runFetch(
  settings: FetchSettings,
  input: RequestInfo | URL,
  init: RequestInit | undefined,
): Promise<Response> {
  const context: FetchContext = { ...settings, requestId: settings.requestIdFactory() };
  const request = await prepareRequest(context, input, init);
  const state: AttemptState = {
    request,
    replaySource: cloneRequest(request),
    useOriginalRequest: true,
    retries: 0,
    unauthorizedReplayUsed: false,
  };
  for (;;) {
    throwIfAborted(state.request.signal, context.requestId);
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

async function sendAttempt(context: FetchContext, state: AttemptState): Promise<Response> {
  const attempt = state.useOriginalRequest ? state.request : state.replaySource?.clone();
  if (attempt === undefined) {
    throw new TypeError('API request body cannot be replayed');
  }
  state.useOriginalRequest = false;
  const response = await context.transport(attempt);
  throwIfAborted(state.request.signal, context.requestId);
  return response;
}

async function handleResponse(
  context: FetchContext,
  state: AttemptState,
  response: Response,
): Promise<AttemptOutcome> {
  if (
    state.replaySource !== undefined &&
    shouldRetryResponse(state.request, response.status, state.retries, context.retry)
  ) {
    await backOff(context, state);
    return RETRY;
  }
  if (response.ok) {
    return response;
  }
  const error = await normalizeResponseError(response);
  if (response.status !== UNAUTHORIZED_STATUS) {
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
  if (cause instanceof ApiError || cause instanceof HttpError) {
    throw cause;
  }
  const aborted = state.request.signal.aborted || isAbortError(cause);
  if (
    !aborted &&
    state.replaySource !== undefined &&
    shouldRetryMethod(state.request, context.retry) &&
    state.retries < context.retry.maxRetries
  ) {
    await backOff(context, state);
    return RETRY;
  }
  throw new NetworkError(
    aborted ? 'API request was aborted' : 'API network request failed',
    context.requestId,
    cause,
    aborted,
  );
}

async function backOff(context: FetchContext, state: AttemptState): Promise<void> {
  await retryDelay(state.retries, context.retry, state.request.signal, context.requestId);
  state.retries += 1;
}

async function decorateRequest(context: FetchContext, request: Request): Promise<Request> {
  const { options, requestId } = context;
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
    return new requestConstructor(new URL(input.toString(), `${baseUrl}/`), init);
  }
  return new requestConstructor(input, init);
}

function createUuid(): string {
  // Runtime probe: React Native and older runtimes may lack `crypto` despite the DOM lib typing.
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

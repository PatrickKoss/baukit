import { isRecord } from '@baukit/type-guards';

import type { DeviceProviderRequestOptions } from './device-flow.js';
import { AuthNodeError } from './errors.js';

export interface JsonResponse {
  readonly response: Response;
  readonly body: Record<string, unknown>;
}

export interface JsonRequest {
  readonly fetchImplementation: typeof globalThis.fetch;
  readonly options: DeviceProviderRequestOptions;
  readonly invalidResponseCode:
    'invalid_device_response' | 'invalid_discovery_document' | 'invalid_token_response';
  readonly requestFailureCode:
    'device_authorization_failed' | 'discovery_failed' | 'refresh_failed';
}

export async function postForm(
  endpoint: string,
  fields: Record<string, string>,
  request: JsonRequest,
): Promise<JsonResponse> {
  return requestJson(
    endpoint,
    {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/x-www-form-urlencoded',
      },
      body: new URLSearchParams(fields),
    },
    request,
  );
}

export async function requestJson(
  input: string,
  init: RequestInit,
  request: JsonRequest,
): Promise<JsonResponse> {
  const { fetchImplementation, options, invalidResponseCode, requestFailureCode } = request;
  const requestTimeout = timeoutSignal(options.signal, options.requestTimeoutMs);
  try {
    const response = await fetchImplementation(input, { ...init, signal: requestTimeout.signal });
    const declaredLength = Number(response.headers.get('content-length'));
    if (Number.isFinite(declaredLength) && declaredLength > options.maxResponseBytes) {
      throw new AuthNodeError(invalidResponseCode);
    }
    const text = await readLimitedText(response, options.maxResponseBytes, invalidResponseCode);
    let value: unknown;
    try {
      value = JSON.parse(text) as unknown;
    } catch {
      throw new AuthNodeError(invalidResponseCode);
    }
    if (!isRecord(value)) throw new AuthNodeError(invalidResponseCode);
    return { response, body: value };
  } catch (error) {
    const mapped = mapAbort(error, options.signal, requestTimeout.didTimeout());
    if (mapped instanceof AuthNodeError) throw mapped;
    throw new AuthNodeError(requestFailureCode, { retryable: true });
  } finally {
    requestTimeout.dispose();
  }
}

async function readLimitedText(
  response: Response,
  maxBytes: number,
  invalidResponseCode: JsonRequest['invalidResponseCode'],
): Promise<string> {
  if (response.body === null) return '';
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = '';
  let bytes = 0;
  for (;;) {
    const chunk = await reader.read();
    if (chunk.done) break;
    bytes += chunk.value.byteLength;
    if (bytes > maxBytes) {
      await reader.cancel().catch(() => undefined);
      throw new AuthNodeError(invalidResponseCode);
    }
    text += decoder.decode(chunk.value, { stream: true });
  }
  return text + decoder.decode();
}

export function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted === true) throw new AuthNodeError('aborted');
}

export function mapAbort(
  error: unknown,
  parentSignal: AbortSignal | undefined,
  timedOut: boolean,
): unknown {
  if (parentSignal?.aborted === true) return new AuthNodeError('aborted');
  if (timedOut) return new AuthNodeError('login_timeout', { retryable: true });
  return error;
}

export function combineSignals(
  first: AbortSignal | undefined,
  second: AbortSignal | undefined,
): AbortSignal | undefined {
  if (first === undefined) return second;
  if (second === undefined) return first;
  return AbortSignal.any([first, second]);
}

export function timeoutSignal(
  parent: AbortSignal | undefined,
  milliseconds: number,
): {
  readonly signal: AbortSignal;
  readonly didTimeout: () => boolean;
  readonly dispose: () => void;
} {
  const controller = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, milliseconds);
  const abortFromParent = (): void => {
    controller.abort();
  };
  parent?.addEventListener('abort', abortFromParent, { once: true });
  if (parent?.aborted === true) controller.abort();
  return {
    signal: controller.signal,
    didTimeout: () => timedOut,
    dispose: () => {
      clearTimeout(timeout);
      parent?.removeEventListener('abort', abortFromParent);
    },
  };
}

export function abortableDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
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

import { isRecord } from '@baukit/type-guards';

import { REQUEST_ID_HEADER } from './headers.js';

/** JSON values accepted in Baukit error details. */
export type JsonValue =
  null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

/** A validated Baukit error envelope. */
export interface ApiErrorEnvelope<Code extends string = string> {
  readonly error: {
    readonly code: Code;
    readonly message: string;
    readonly requestId: string;
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
    this.requestId = envelope.error.requestId;
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
    typeof error['requestId'] !== 'string' ||
    !isJsonObject(error['details'])
  ) {
    return null;
  }

  return new ApiError(
    {
      error: {
        code: error['code'],
        message: error['message'],
        requestId: error['requestId'],
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

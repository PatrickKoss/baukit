# `@baukit/api-runtime`

Shared client-side behavior for Baukit product APIs: explicit environment selection, bearer-token and request-ID headers, optional W3C trace propagation, normalized errors, safe retries, and a test fetch transport. It works in browsers and React Native and never reads `process.env`.

## Generated client usage

The package wraps `openapi-fetch`; each product continues to own its generated `paths` type.

```ts
import { createApiClient, resolveApiEnvironment } from '@baukit/api-runtime';
import type { paths } from './generated/api.js';

const environment = resolveApiEnvironment('production', {
  development: 'http://localhost:3000',
  production: 'https://api.example.com',
});

export const api = createApiClient<paths>({
  ...environment,
  tokenProvider: async () => session.accessToken ?? null,
  onUnauthorized: async ({ canRetry }) => {
    if (!canRetry) return 'handled';
    const token = await session.accessToken({ forceRefresh: true });
    return token === undefined ? 'handled' : 'retry-once';
  },
});

const { data } = await api.GET('/widgets');
```

If a generated wrapper already creates the client, pass it a configured runtime instead:

```ts
import createClient from 'openapi-fetch';
import { createApiRuntime } from '@baukit/api-runtime';

const runtime = createApiRuntime({
  baseUrl: 'https://api.example.com',
  environment: 'production',
  tokenProvider: getAccessToken,
});

const api = createClient<paths>({ baseUrl: runtime.baseUrl, fetch: runtime.fetch });
```

`tokenProvider` runs for every logical request; the runtime does not cache its result. Every request receives a new UUID in `x-request-id`. Configure `traceparentProvider` only when the product has a tracing implementation.

## Unauthorized recovery

`onUnauthorized` remains a notification hook when it returns `void` or `'handled'`: the normalized 401 is thrown as before. Returning `'retry-once'` explicitly asks the runtime to reacquire credentials through `tokenProvider` and replay a preflighted request clone. The hook receives `canRetry: false` when the body cannot be cloned; returning `'retry-once'` then safely falls back to the original 401. Recovery runs at most once, a second 401 stops, and the original `AbortSignal` remains effective during refresh and replay.

Set `onUnauthorizedExhausted` when the product must stop schedulers or move to
a signed-out state after that replay also returns 401. It receives the final
normalized error with `canRetry: false`; observer failures never replace the
API failure.

This handshake does not make arbitrary mutations safe. Replaying `POST`, `PATCH`, or any write whose outcome may already have committed still requires a product/server idempotency contract. Follow the repository's [integration reliability recipe](../../../docs/platform/integration-reliability.md), [offline replay contract](../../../docs/platform/offline-readiness-contract.md), and [add-endpoint replay/idempotency guidance](../../../agent-skills/skills/baukit-add-endpoint/SKILL.md).

## Error handling

Non-success responses throw one of three typed errors. Raw fetch/CORS errors are always wrapped.

```ts
import { isApiError, isHttpError, isNetworkError } from '@baukit/api-runtime';

try {
  await api.POST('/widgets', { body: widget });
} catch (error) {
  if (isApiError(error, 'validation_failed')) {
    showValidation(error.details, error.requestId);
  } else if (isNetworkError(error)) {
    showOfflineMessage();
  } else if (isHttpError(error)) {
    reportUnexpectedResponse(error.status, error.requestId);
  } else {
    throw error;
  }
}
```

- `ApiError`: a valid Baukit `{ error: { code, message, request_id, details } }` envelope.
- `HttpError`: an HTTP failure with a missing or malformed envelope, including non-JSON bodies.
- `NetworkError`: no HTTP response, such as an offline, DNS, fetch, or CORS failure. Aborts set `aborted` to `true`.

The backend `message` is public, safe fallback text. Localized clients should resolve `ApiError.code` plus structured `ApiError.details` through their product catalog and use `ApiError.message` only when that resolution is unavailable. Do not parse the message or use it as a stable localization key.

## Retry semantics

Retries use exponential backoff with full jitter, capped by `maxDelayMs`. The same request ID is retained across attempts.

| Request/result                | Default          | Configurable                                 |
| ----------------------------- | ---------------- | -------------------------------------------- |
| `GET`, `HEAD` + network error | Retry            | `maxRetries`, delays, or disable             |
| `GET`, `HEAD` + 502/503/504   | Retry            | `maxRetries`, delays, or disable             |
| `OPTIONS`, `PUT`, `DELETE`    | No retry         | May be explicitly opted in through `methods` |
| Any 4xx                       | Never            | No                                           |
| `POST`, `PATCH`               | Never            | No                                           |
| Abort                         | Stop immediately | No                                           |

Defaults are two retries, a 100 ms initial ceiling, and a 2,000 ms maximum ceiling. Use `retry: false` to disable retries.

## Tests

`MockFetch` queues responses, errors, or handlers and records cloned requests:

```ts
const mock = new MockFetch().enqueueJson({ widgets: [] });
const runtime = createApiRuntime({
  baseUrl: 'https://api.example.test',
  environment: 'test',
  fetch: mock.fetch,
});

await runtime.fetch('/widgets');
mock.assertRequest(0, { method: 'GET', url: 'https://api.example.test/widgets' });
mock.assertQueueEmpty();
```

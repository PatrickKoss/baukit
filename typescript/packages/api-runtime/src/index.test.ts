import { describe, expect, it, vi } from 'vitest';

import {
  ApiError,
  HttpError,
  MockFetch,
  NetworkError,
  REQUEST_ID_HEADER,
  createApiFetch,
  createApiRuntime,
  isApiError,
  isHttpError,
  isNetworkError,
  normalizeResponseError,
  parseApiErrorEnvelope,
  resolveApiEnvironment,
} from './index.js';

const baseOptions = { baseUrl: 'https://api.example.test/v1', environment: 'test' } as const;
const rustEnvelopeSample = {
  error: {
    code: 'validation_failed',
    message: 'The request is invalid',
    request_id: 'req-123',
    details: {},
  },
};

describe('configuration', () => {
  it('resolves a caller-provided environment map without ambient environment access', () => {
    expect(
      resolveApiEnvironment('staging', {
        production: 'https://api.example.test',
        staging: 'https://staging.example.test/',
      }),
    ).toEqual({ baseUrl: 'https://staging.example.test', environment: 'staging' });
    expect(() => resolveApiEnvironment('preview', {})).toThrow('Unknown API environment: preview');
  });

  it('resolves standalone relative paths against the configured base URL', async () => {
    const mock = new MockFetch().enqueueJson({ ok: true });
    const runtime = createApiRuntime({ ...baseOptions, fetch: mock.fetch, retry: false });
    await runtime.fetch('widgets');
    mock.assertRequest(0, { url: 'https://api.example.test/v1/widgets' });
  });
});

describe('Baukit error normalization', () => {
  it('parses the serialized Rust envelope shape', () => {
    const error = parseApiErrorEnvelope(rustEnvelopeSample, 400);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      code: 'validation_failed',
      details: {},
      kind: 'api',
      message: 'The request is invalid',
      requestId: 'req-123',
      status: 400,
    });
    expect(isApiError(error, 'validation_failed')).toBe(true);
    expect(isApiError(error, 'not_found')).toBe(false);
  });

  it('rejects lookalike and malformed envelopes', () => {
    expect(parseApiErrorEnvelope({ error: { ...rustEnvelopeSample.error } }, 400)).toBeInstanceOf(
      ApiError,
    );
    expect(
      parseApiErrorEnvelope({ error: { ...rustEnvelopeSample.error, requestId: 'wrong' } }, 400),
    ).not.toBeNull();
    expect(
      parseApiErrorEnvelope(
        { error: { code: 'bad', message: 'Bad', requestId: 'wrong', details: {} } },
        400,
      ),
    ).toBeNull();
    expect(
      parseApiErrorEnvelope({ error: { ...rustEnvelopeSample.error, details: [] } }, 400),
    ).toBeNull();
  });

  it('normalizes non-envelope JSON and non-JSON responses as HttpError', async () => {
    const jsonError = await normalizeResponseError(
      new Response(JSON.stringify({ message: 'upstream failed' }), {
        status: 500,
        headers: { [REQUEST_ID_HEADER]: 'header-request' },
      }),
    );
    expect(jsonError).toBeInstanceOf(HttpError);
    expect(jsonError).toMatchObject({
      body: { message: 'upstream failed' },
      kind: 'http',
      requestId: 'header-request',
      status: 500,
    });

    const textError = await normalizeResponseError(new Response('not json', { status: 502 }));
    expect(isHttpError(textError)).toBe(true);
    expect(textError.body).toBe('not json');
  });

  it('normalizes transport failures without exposing raw fetch errors', async () => {
    const raw = new TypeError('Failed to fetch');
    const mock = new MockFetch().enqueue(raw);
    const apiFetch = createApiFetch({ ...baseOptions, fetch: mock.fetch, retry: false });

    const caught = await apiFetch('/widgets').catch((error: unknown) => error);
    expect(caught).toBeInstanceOf(NetworkError);
    expect(isNetworkError(caught)).toBe(true);
    expect(caught).toMatchObject({ aborted: false, cause: raw, kind: 'network' });
  });
});

describe('request decoration', () => {
  it('injects a fresh bearer token and request ID per request', async () => {
    const mock = new MockFetch().enqueueJson({}).enqueueJson({});
    const tokenProvider = vi
      .fn()
      .mockResolvedValueOnce('first-token')
      .mockResolvedValueOnce('second-token');
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      retry: false,
      tokenProvider,
    });

    await apiFetch('/one');
    await apiFetch('/two');

    expect(tokenProvider).toHaveBeenCalledTimes(2);
    expect(mock.request(0).headers.get('authorization')).toBe('Bearer first-token');
    expect(mock.request(1).headers.get('authorization')).toBe('Bearer second-token');
    const firstId = mock.request(0).headers.get(REQUEST_ID_HEADER);
    const secondId = mock.request(1).headers.get(REQUEST_ID_HEADER);
    expect(firstId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
    );
    expect(secondId).not.toBe(firstId);
  });

  it('does not inject authorization for a null token', async () => {
    const mock = new MockFetch().enqueueJson({});
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      retry: false,
      tokenProvider: () => Promise.resolve(null),
    });

    await apiFetch('/anonymous');
    expect(mock.request(0).headers.has('authorization')).toBe(false);
  });

  it('injects traceparent only when its optional provider is configured', async () => {
    const mock = new MockFetch().enqueueJson({}).enqueueJson({});
    await createApiFetch({ ...baseOptions, fetch: mock.fetch, retry: false })('/without-trace');
    await createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      retry: false,
      traceparentProvider: () => '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01',
    })('/with-trace');

    expect(mock.request(0).headers.has('traceparent')).toBe(false);
    expect(mock.request(1).headers.get('traceparent')).toBe(
      '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01',
    );
  });

  it('fires the 401 hook with the normalized error before rejecting', async () => {
    const mock = new MockFetch().enqueueJson(rustEnvelopeSample, { status: 401 });
    const onUnauthorized = vi.fn();
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      onUnauthorized,
      retry: false,
    });

    await expect(apiFetch('/private')).rejects.toMatchObject({
      code: 'validation_failed',
      status: 401,
    });
    expect(onUnauthorized).toHaveBeenCalledOnce();
    expect(onUnauthorized).toHaveBeenCalledWith(
      expect.objectContaining({ error: expect.any(ApiError) as ApiError }),
    );
  });
});

describe('retry policy', () => {
  it('retries GET network errors and 502/503/504 using bounded exponential full jitter', async () => {
    const delays: number[] = [];
    const mock = new MockFetch()
      .enqueue(new TypeError('offline'))
      .enqueue(new Response(null, { status: 503 }))
      .enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      retry: {
        baseDelayMs: 100,
        maxDelayMs: 150,
        maxRetries: 2,
        random: () => 1,
        sleep: (delay) => {
          delays.push(delay);
          return Promise.resolve();
        },
      },
    });

    await expect(apiFetch('/eventually')).resolves.toHaveProperty('status', 200);
    expect(mock.requests).toHaveLength(3);
    expect(delays).toEqual([100, 150]);
    expect(
      new Set(mock.requests.map((request) => request.headers.get(REQUEST_ID_HEADER))).size,
    ).toBe(1);
  });

  it.each(['POST', 'PATCH'] as const)(
    'never retries %s even when configuration is bypassed',
    async (method) => {
      const mock = new MockFetch().enqueue(new TypeError('offline')).enqueueJson({ ok: true });
      const apiFetch = createApiFetch({
        ...baseOptions,
        fetch: mock.fetch,
        retry: { methods: ['GET', method] as readonly ('GET' | 'POST' | 'PATCH')[] } as never,
      });

      await expect(apiFetch('/write', { method })).rejects.toBeInstanceOf(NetworkError);
      expect(mock.requests).toHaveLength(1);
    },
  );

  it('defaults to GET/HEAD and permits explicit idempotent-method opt-in', async () => {
    const defaultMock = new MockFetch()
      .enqueue(new Response(null, { status: 503 }))
      .enqueueJson({});
    await expect(
      createApiFetch({ ...baseOptions, fetch: defaultMock.fetch })('/put', { method: 'PUT' }),
    ).rejects.toMatchObject({ status: 503 });
    expect(defaultMock.requests).toHaveLength(1);

    const optInMock = new MockFetch().enqueue(new Response(null, { status: 504 })).enqueueJson({});
    await expect(
      createApiFetch({
        ...baseOptions,
        fetch: optInMock.fetch,
        retry: { baseDelayMs: 0, methods: ['PUT'] },
      })('/put', { method: 'PUT' }),
    ).resolves.toHaveProperty('status', 200);
    expect(optInMock.requests).toHaveLength(2);
  });

  it('never retries 4xx responses', async () => {
    const mock = new MockFetch().enqueueJson(rustEnvelopeSample, { status: 429 }).enqueueJson({});
    const apiFetch = createApiFetch({ ...baseOptions, fetch: mock.fetch });

    await expect(apiFetch('/limited')).rejects.toBeInstanceOf(ApiError);
    expect(mock.requests).toHaveLength(1);
  });

  it('aborts during backoff and does not make another attempt', async () => {
    const controller = new AbortController();
    const mock = new MockFetch().enqueue(new Response(null, { status: 503 })).enqueueJson({});
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      retry: {
        sleep: async (_delay, signal) => {
          await Promise.resolve();
          controller.abort('stop');
          signal?.throwIfAborted();
        },
      },
    });

    const caught = await apiFetch('/slow', { signal: controller.signal }).catch(
      (error: unknown) => error,
    );
    expect(caught).toBeInstanceOf(NetworkError);
    expect(caught).toMatchObject({ aborted: true });
    expect(mock.requests).toHaveLength(1);
  });
});

describe('MockFetch assertions', () => {
  it('queues responses and asserts recorded requests', async () => {
    const mock = new MockFetch().enqueueJson({ ok: true });
    await mock.fetch('https://api.example.test/ping', { headers: { 'x-test': 'yes' } });
    expect(() => {
      mock.assertRequest(0, {
        headers: { 'x-test': 'yes' },
        method: 'GET',
        url: 'https://api.example.test/ping',
      });
    }).not.toThrow();
    expect(() => {
      mock.assertQueueEmpty();
    }).not.toThrow();
  });
});

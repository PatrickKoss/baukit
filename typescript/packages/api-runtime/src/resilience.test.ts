import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  ApiUnavailableError,
  HttpError,
  IDEMPOTENCY_KEY_HEADER,
  MockFetch,
  NetworkError,
  createApiFetch,
  isApiUnavailableError,
  type FetchImplementation,
} from './index.js';

const baseOptions = { baseUrl: 'https://api.example.test/v1', environment: 'test' } as const;
const START = Date.UTC(2026, 0, 1);

function manualClock(): { now: () => number; advance: (ms: number) => void } {
  let current = START;
  return {
    now: () => current,
    advance: (ms) => {
      current += ms;
    },
  };
}

function clockedRetry(clock: ReturnType<typeof manualClock>, delays: number[]) {
  return {
    random: () => 1,
    sleep: (delayMs: number) => {
      delays.push(delayMs);
      clock.advance(delayMs);
      return Promise.resolve();
    },
  };
}

function hangingFetch(): FetchImplementation {
  return (_input, init) =>
    new Promise((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => {
        reject(new DOMException('aborted', 'AbortError'));
      });
    });
}

afterEach(() => {
  vi.useRealTimers();
});

describe('request timeout', () => {
  it('fails a hanging attempt after requestTimeoutMs as a non-aborted network error', async () => {
    vi.useFakeTimers();
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: hangingFetch(),
      requestTimeoutMs: 1_000,
      retry: false,
    });

    const pending = apiFetch('/hang').catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(1_000);
    const caught = await pending;

    expect(caught).toBeInstanceOf(NetworkError);
    expect(caught).toMatchObject({ aborted: false, message: 'API request timed out' });
  });

  it('defaults to a 30 second timeout', async () => {
    vi.useFakeTimers();
    const apiFetch = createApiFetch({ ...baseOptions, fetch: hangingFetch(), retry: false });
    let settled = false;
    const pending = apiFetch('/hang').catch((error: unknown) => {
      settled = true;
      return error;
    });

    await vi.advanceTimersByTimeAsync(29_999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await expect(pending).resolves.toMatchObject({ message: 'API request timed out' });
  });

  it('still honours the caller signal and reports it as aborted', async () => {
    const controller = new AbortController();
    const apiFetch = createApiFetch({ ...baseOptions, fetch: hangingFetch(), retry: false });
    const pending = apiFetch('/hang', { signal: controller.signal }).catch(
      (error: unknown) => error,
    );
    controller.abort('stop');

    await expect(pending).resolves.toMatchObject({ aborted: true });
  });

  it('retries a timed-out GET', async () => {
    vi.useFakeTimers();
    let calls = 0;
    const fetch: FetchImplementation = (input, init) => {
      calls += 1;
      return calls === 1 ? hangingFetch()(input, init) : Promise.resolve(new Response('{}'));
    };
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch,
      requestTimeoutMs: 100,
      retry: { baseDelayMs: 0 },
    });

    const pending = apiFetch('/slow');
    await vi.advanceTimersByTimeAsync(101);
    await expect(pending).resolves.toHaveProperty('status', 200);
    expect(calls).toBe(2);
  });
});

describe('Retry-After', () => {
  it.each([429, 503])('retries a GET %i after the delay-seconds value', async (status) => {
    const clock = manualClock();
    const delays: number[] = [];
    const mock = new MockFetch()
      .enqueue(new Response(null, { status, headers: { 'retry-after': '3' } }))
      .enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: clockedRetry(clock, delays),
    });

    await expect(apiFetch('/limited')).resolves.toHaveProperty('status', 200);
    expect(delays).toEqual([3_000]);
  });

  it('accepts an HTTP date and clamps to maxRetryAfterMs', async () => {
    const clock = manualClock();
    const delays: number[] = [];
    const inTwoSeconds = new Date(START + 2_000).toUTCString();
    const inAnHour = new Date(START + 3_600_000).toUTCString();
    const mock = new MockFetch()
      .enqueue(new Response(null, { status: 503, headers: { 'retry-after': inTwoSeconds } }))
      .enqueue(new Response(null, { status: 429, headers: { 'retry-after': inAnHour } }))
      .enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: { ...clockedRetry(clock, delays), maxRetryAfterMs: 10_000 },
    });

    await expect(apiFetch('/limited')).resolves.toHaveProperty('status', 200);
    expect(delays).toEqual([2_000, 10_000]);
  });

  it('does not retry an unkeyed POST but still starts the cooldown', async () => {
    const clock = manualClock();
    const mock = new MockFetch().enqueue(
      new Response(null, { status: 429, headers: { 'retry-after': '5' } }),
    );
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: { methods: ['GET', 'POST'] },
    });

    await expect(apiFetch('/write', { method: 'POST' })).rejects.toBeInstanceOf(HttpError);
    const caught = await apiFetch('/read').catch((error: unknown) => error);
    expect(caught).toBeInstanceOf(ApiUnavailableError);
    expect(caught).toMatchObject({ retryAt: START + 5_000 });
    expect(mock.requests).toHaveLength(1);
  });

  it('retries a keyed POST that opted in', async () => {
    const clock = manualClock();
    const mock = new MockFetch()
      .enqueue(new Response(null, { status: 429, headers: { 'retry-after': '1' } }))
      .enqueueJson({}, { status: 201 });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: { ...clockedRetry(clock, []), methods: ['POST'] },
    });

    await expect(
      apiFetch('/write', { method: 'POST', headers: { [IDEMPOTENCY_KEY_HEADER]: 'k' } }),
    ).resolves.toHaveProperty('status', 201);
    expect(mock.requests).toHaveLength(2);
  });
});

describe('per-origin cooldown', () => {
  it('turns 50 concurrent calls after a 503 with Retry-After: 5 into no extra requests', async () => {
    vi.useFakeTimers({ now: START });
    let sent = 0;
    const fetch: FetchImplementation = () => {
      sent += 1;
      return Promise.resolve(
        sent === 1
          ? new Response(null, { status: 503, headers: { 'retry-after': '5' } })
          : new Response('{}'),
      );
    };
    const apiFetch = createApiFetch({ ...baseOptions, fetch });

    const first = apiFetch('/feed');
    await vi.advanceTimersByTimeAsync(0);
    const results = await Promise.allSettled(Array.from({ length: 50 }, () => apiFetch('/feed')));

    expect(sent).toBe(1);
    expect(results.every((result) => result.status === 'rejected')).toBe(true);
    expect(
      results.every(
        (result) => result.status === 'rejected' && isApiUnavailableError(result.reason),
      ),
    ).toBe(true);

    await vi.advanceTimersByTimeAsync(5_000);
    await expect(first).resolves.toHaveProperty('status', 200);
    await expect(apiFetch('/feed')).resolves.toHaveProperty('status', 200);
    expect(sent).toBe(3);
  });

  it('opens after 5 consecutive network failures and fails fast until the cooldown ends', async () => {
    const clock = manualClock();
    const mock = new MockFetch();
    for (let index = 0; index < 5; index += 1) {
      mock.enqueue(new TypeError('offline'));
    }
    mock.enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: false,
      cooldown: { networkFailureCooldownMs: 10_000, random: () => 0 },
    });

    for (let index = 0; index < 5; index += 1) {
      await expect(apiFetch('/down')).rejects.toBeInstanceOf(NetworkError);
    }
    await expect(apiFetch('/down')).rejects.toMatchObject({ retryAt: START + 10_000 });
    expect(mock.requests).toHaveLength(5);

    clock.advance(10_000);
    await expect(apiFetch('/down')).resolves.toHaveProperty('status', 200);
  });

  it('resets the failure count on any response', async () => {
    const clock = manualClock();
    const mock = new MockFetch();
    for (let index = 0; index < 4; index += 1) {
      mock.enqueue(new TypeError('offline'));
    }
    mock.enqueue(new Response(null, { status: 500 }));
    for (let index = 0; index < 4; index += 1) {
      mock.enqueue(new TypeError('offline'));
    }
    mock.enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: false,
    });

    for (let index = 0; index < 9; index += 1) {
      await apiFetch('/flaky').catch(() => undefined);
    }
    await expect(apiFetch('/flaky')).resolves.toHaveProperty('status', 200);
  });

  it('does not count caller aborts as network failures', async () => {
    const clock = manualClock();
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: hangingFetch(),
      now: clock.now,
      retry: false,
      cooldown: { networkFailureThreshold: 1, random: () => 0 },
    });
    const controller = new AbortController();
    const pending = apiFetch('/hang', { signal: controller.signal }).catch(
      (error: unknown) => error,
    );
    controller.abort();
    await expect(pending).resolves.toMatchObject({ aborted: true });

    const mock = new MockFetch().enqueueJson({});
    const next = createApiFetch({ ...baseOptions, fetch: mock.fetch, now: clock.now });
    await expect(next('/ok')).resolves.toHaveProperty('status', 200);
  });

  it('fails fast instead of sleeping through a network cooldown during retries', async () => {
    const clock = manualClock();
    const delays: number[] = [];
    const mock = new MockFetch().enqueue(new TypeError('offline')).enqueueJson({});
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: clockedRetry(clock, delays),
      cooldown: { networkFailureThreshold: 1, random: () => 0 },
    });

    await expect(apiFetch('/down')).rejects.toBeInstanceOf(ApiUnavailableError);
    expect(delays).toEqual([]);
    expect(mock.requests).toHaveLength(1);
  });

  it('keeps other origins callable', async () => {
    const clock = manualClock();
    const mock = new MockFetch()
      .enqueue(new Response(null, { status: 503, headers: { 'retry-after': '60' } }))
      .enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: false,
    });

    await expect(apiFetch('/a')).rejects.toBeInstanceOf(HttpError);
    await expect(apiFetch('https://other.example.test/b')).resolves.toHaveProperty('status', 200);
    await expect(apiFetch('/a')).rejects.toBeInstanceOf(ApiUnavailableError);
  });

  it('can be disabled', async () => {
    const clock = manualClock();
    const mock = new MockFetch()
      .enqueue(new Response(null, { status: 503, headers: { 'retry-after': '60' } }))
      .enqueueJson({ ok: true });
    const apiFetch = createApiFetch({
      ...baseOptions,
      fetch: mock.fetch,
      now: clock.now,
      retry: false,
      cooldown: false,
    });

    await expect(apiFetch('/a')).rejects.toBeInstanceOf(HttpError);
    await expect(apiFetch('/a')).resolves.toHaveProperty('status', 200);
  });
});

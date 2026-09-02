import { describe, expect, it, vi } from 'vitest';

import {
  OAuthSessionCoordinator,
  createReturnUrlValidator,
  type BrowserPopupSession,
  type OAuthClock,
  type OAuthInFlightSession,
  type OAuthSessionResult,
  type OAuthSessionStorage,
} from './oauth-session.js';

const RETURN_URL = 'https://app.test/oauth/callback';
const CALLBACK_URL = `${RETURN_URL}?code=authorization-code&state=nonce-1`;

describe('OAuth session coordinator', () => {
  it('reserves a browser popup before waiting for the authorization URL', async () => {
    const authorization = deferred<string>();
    const result = deferred<OAuthSessionResult>();
    const navigate = vi.fn();
    const popup = popupSession(result.promise, { navigate });
    const browser = { openPlaceholder: vi.fn(() => popup) };
    const fixture = coordinator({ browser });

    const outcome = fixture.value.authorize({
      platform: 'web',
      returnUrl: RETURN_URL,
      createAuthorizationUrl: vi.fn(() => authorization.promise),
    });

    expect(browser.openPlaceholder).toHaveBeenCalledWith({
      placeholderUrl: 'about:blank',
      returnUrl: RETURN_URL,
      windowName: 'baukit-integration-oauth',
    });
    expect(navigate).not.toHaveBeenCalled();

    authorization.resolve('https://accounts.test/authorize');
    await settle();
    expect(navigate).toHaveBeenCalledWith('https://accounts.test/authorize');
    result.resolve({ type: 'success', returnUrl: CALLBACK_URL });

    await expect(outcome).resolves.toEqual({
      type: 'success',
      callbackParams: { code: 'authorization-code', state: 'nonce-1' },
    });
    expect(fixture.storage.value).toBeNull();
  });

  it('runs a native auth session after the authorization URL is ready', async () => {
    const run = vi.fn(() => Promise.resolve({ type: 'success', returnUrl: CALLBACK_URL } as const));
    const native = {
      run,
      cancel: vi.fn(),
    };
    const fixture = coordinator({ native });

    await expect(
      fixture.value.authorize({
        platform: 'native',
        returnUrl: RETURN_URL,
        createAuthorizationUrl: () => Promise.resolve('https://accounts.test/authorize'),
      }),
    ).resolves.toMatchObject({ type: 'success' });
    expect(run).toHaveBeenCalledWith({
      authorizationUrl: 'https://accounts.test/authorize',
      returnUrl: RETURN_URL,
    });
  });

  it('returns cancelled and clears its in-flight state', async () => {
    const redirectAuthorize = vi.fn(() => Promise.resolve({ type: 'cancelled' } as const));
    const redirect = {
      authorize: redirectAuthorize,
    };
    const fixture = coordinator({
      browser: { openPlaceholder: () => null },
      redirect,
    });

    await expect(
      fixture.value.authorize({
        platform: 'web',
        returnUrl: RETURN_URL,
        createAuthorizationUrl: () => Promise.resolve('https://accounts.test/authorize'),
      }),
    ).resolves.toEqual({ type: 'cancelled' });
    expect(redirectAuthorize).toHaveBeenCalledWith({
      authorizationUrl: 'https://accounts.test/authorize',
      returnUrl: RETURN_URL,
    });
    expect(fixture.storage.value).toBeNull();
  });

  it('times out a stalled authorization start and closes its reserved popup', async () => {
    const close = vi.fn();
    const navigate = vi.fn();
    const popup = popupSession(Promise.resolve({ type: 'cancelled' }), { close, navigate });
    const fixture = coordinator({ browser: { openPlaceholder: () => popup } });
    const outcome = fixture.value.authorize({
      platform: 'web',
      returnUrl: RETURN_URL,
      createAuthorizationUrl: () => new Promise<string>(() => undefined),
    });
    await settle();

    fixture.clock.advance(1_000);

    await expect(outcome).resolves.toEqual({ type: 'timed_out' });
    expect(close).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();
    expect(fixture.storage.value).toBeNull();
  });

  it('rejects a return from another origin or path', async () => {
    const fixture = coordinator();
    await expect(
      fixture.value.handleRedirect('https://app.test/oauth/other?state=nonce-1'),
    ).resolves.toEqual({ type: 'invalid_return' });
    await expect(
      fixture.value.handleRedirect('https://app.test.evil/oauth/callback?state=nonce-1'),
    ).resolves.toEqual({ type: 'invalid_return' });
  });

  it('returns storage mismatch when the callback state differs', async () => {
    const storage = new MemoryStorage({
      stateNonce: 'stored-nonce',
      returnUrl: RETURN_URL,
      createdAt: 0,
    });
    const fixture = coordinator({ storage });

    await expect(fixture.value.handleRedirect(CALLBACK_URL)).resolves.toEqual({
      type: 'storage_mismatch',
    });
    expect(storage.value?.stateNonce).toBe('stored-nonce');
  });

  it('handles a valid redirect after a same-tab navigation', async () => {
    const storage = new MemoryStorage({
      stateNonce: 'nonce-1',
      returnUrl: RETURN_URL,
      createdAt: 0,
    });
    const fixture = coordinator({ storage });

    await expect(fixture.value.handleRedirect(CALLBACK_URL)).resolves.toEqual({
      type: 'success',
      callbackParams: { code: 'authorization-code', state: 'nonce-1' },
    });
  });

  it('rejects duplicate callback parameters', async () => {
    const storage = new MemoryStorage({
      stateNonce: 'nonce-1',
      returnUrl: RETURN_URL,
      createdAt: 0,
    });
    const fixture = coordinator({ storage });

    await expect(fixture.value.handleRedirect(`${CALLBACK_URL}&state=nonce-1`)).resolves.toEqual({
      type: 'invalid_return',
    });
  });

  it('discards a stale in-flight session before saving a new one', async () => {
    const storage = new MemoryStorage({
      stateNonce: 'old-nonce',
      returnUrl: RETURN_URL,
      createdAt: 0,
    });
    const clock = new ControlledClock(2_000);
    const redirect = { authorize: () => Promise.resolve({ type: 'cancelled' } as const) };
    const fixture = coordinator({ storage, clock, redirect });

    await fixture.value.authorize({
      platform: 'web',
      returnUrl: RETURN_URL,
      createAuthorizationUrl: () => Promise.resolve('https://accounts.test/authorize'),
    });

    expect(storage.events.slice(0, 3)).toEqual(['load', 'clear', 'save:nonce-1']);
    expect(storage.value).toBeNull();
  });

  it('validates an exact custom-scheme authority and allowlisted path', () => {
    const validate = createReturnUrlValidator({
      origin: 'sample-app://oauth',
      paths: ['/callback'],
    });
    expect(validate('sample-app://oauth/callback?state=one')).toBe(true);
    expect(validate('sample-app://other/callback?state=one')).toBe(false);
    expect(validate('sample-app://oauth/callback/extra?state=one')).toBe(false);
  });
});

class MemoryStorage implements OAuthSessionStorage {
  public readonly events: string[] = [];

  public constructor(public value: OAuthInFlightSession | null = null) {}

  public load(): Promise<OAuthInFlightSession | null> {
    this.events.push('load');
    return Promise.resolve(this.value);
  }

  public save(session: OAuthInFlightSession): Promise<void> {
    this.events.push(`save:${session.stateNonce}`);
    this.value = session;
    return Promise.resolve();
  }

  public clear(): Promise<void> {
    this.events.push('clear');
    this.value = null;
    return Promise.resolve();
  }
}

class ControlledClock implements OAuthClock {
  readonly #timers = new Map<object, { callback: () => void; dueAt: number }>();

  public constructor(private currentTime = 0) {}

  public now(): number {
    return this.currentTime;
  }

  public setTimeout(callback: () => void, timeoutMs: number): object {
    const handle = {};
    this.#timers.set(handle, { callback, dueAt: this.currentTime + timeoutMs });
    return handle;
  }

  public clearTimeout(handle: unknown): void {
    if (typeof handle === 'object' && handle !== null) this.#timers.delete(handle);
  }

  public advance(milliseconds: number): void {
    this.currentTime += milliseconds;
    for (const [handle, timer] of this.#timers) {
      if (timer.dueAt > this.currentTime) continue;
      this.#timers.delete(handle);
      timer.callback();
    }
  }
}

function coordinator(
  options: Partial<ConstructorParameters<typeof OAuthSessionCoordinator>[0]> = {},
) {
  const storage = options.storage ?? new MemoryStorage();
  const clock = options.clock ?? new ControlledClock();
  return {
    storage,
    clock: clock as ControlledClock,
    value: new OAuthSessionCoordinator({
      storage,
      clock,
      timeoutMs: 1_000,
      createStateNonce: () => 'nonce-1',
      validateReturnUrl: createReturnUrlValidator({
        origin: 'https://app.test',
        paths: ['/oauth/callback'],
      }),
      ...options,
    }),
  };
}

function popupSession(
  result: Promise<OAuthSessionResult>,
  callbacks: {
    readonly navigate?: (authorizationUrl: string) => void;
    readonly close?: () => void;
  } = {},
): BrowserPopupSession {
  return {
    result,
    navigate: callbacks.navigate ?? vi.fn(),
    close: callbacks.close ?? vi.fn(),
  };
}

function deferred<T>() {
  let resolveValue: (value: T) => void = () => undefined;
  const promise = new Promise<T>((resolve) => {
    resolveValue = resolve;
  });
  return { promise, resolve: resolveValue };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

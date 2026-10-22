// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { CALLBACK_MESSAGE, completeSilentTokenCallback } from './index.js';
import { openSilentAuthorizationWindow } from './silent-token-browser.js';

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

it.each([false, true])(
  'checks origin and source for interactive=%s and cleans up listeners',
  async (interactive) => {
    const target = { location: { replace: vi.fn() }, close: vi.fn() };
    const targetWindow = target as unknown as Window;
    vi.spyOn(HTMLIFrameElement.prototype, 'contentWindow', 'get').mockReturnValue(targetWindow);
    const open = vi.spyOn(window, 'open').mockReturnValue(targetWindow);
    const remove = vi.spyOn(window, 'removeEventListener');
    const storage = vi.spyOn(Storage.prototype, 'setItem');
    const authorization = openSilentAuthorizationWindow(window, interactive);
    expect(open).toHaveBeenCalledTimes(interactive ? 1 : 0);
    if (!interactive) expect(document.querySelector('iframe')?.hidden).toBe(true);
    const url = new URL('https://identity.example/auth');
    const controller = new AbortController();
    let resolved = false;
    const pending = authorization.receive(url, controller.signal).then((value) => {
      resolved = true;
      return value;
    });
    const valid = {
      type: 'baukit-silent-token-callback',
      url: `${window.location.origin}/callback?code=one`,
    };
    for (const data of [
      null,
      'bad',
      {},
      { type: 'wrong', url: 'x' },
      { type: valid.type, url: 42 },
    ]) {
      window.dispatchEvent(
        new MessageEvent('message', { origin: window.location.origin, source: targetWindow, data }),
      );
    }
    window.dispatchEvent(
      new MessageEvent('message', {
        origin: 'https://evil.example',
        source: targetWindow,
        data: valid,
      }),
    );
    window.dispatchEvent(
      new MessageEvent('message', { origin: window.location.origin, source: window, data: valid }),
    );
    await Promise.resolve();
    expect(resolved).toBe(false);
    window.dispatchEvent(
      new MessageEvent('message', {
        origin: window.location.origin,
        source: targetWindow,
        data: valid,
      }),
    );
    expect(await pending).toBe(valid.url);
    expect(target.location.replace).toHaveBeenCalledWith(url.toString());
    expect(remove).toHaveBeenCalledWith('message', expect.any(Function));
    authorization.close();
    expect(document.querySelector('iframe')).toBeNull();
    expect(target.close).toHaveBeenCalledTimes(interactive ? 1 : 0);
    expect(storage).not.toHaveBeenCalled();
  },
);

it('opens an explicit window synchronously and aborts its message listener', async () => {
  const popup = { location: { replace: vi.fn() }, close: vi.fn() };
  const open = vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
  const target = openSilentAuthorizationWindow(window, true);
  expect(open).toHaveBeenCalledTimes(1);
  const controller = new AbortController();
  const pending = target.receive(new URL('https://identity.example/auth'), controller.signal);
  controller.abort(new Error('cancelled'));
  await expect(pending).rejects.toThrow('cancelled');
  target.close();
  expect(popup.close).toHaveBeenCalled();
});

it('exposes the live popup closed state', () => {
  const popup = { closed: false, close: vi.fn() };
  vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
  const target = openSilentAuthorizationWindow(window, true);
  expect(target.closed).toBe(false);
  popup.closed = true;
  expect(target.closed).toBe(true);
  target.close();
});

it('reports blocked popups and handles pre-aborted authorization without navigating', async () => {
  vi.spyOn(window, 'open').mockReturnValue(null);
  expect(() => openSilentAuthorizationWindow(window, true)).toThrow('popup_blocked');
  const replace = vi.fn();
  vi.spyOn(HTMLIFrameElement.prototype, 'contentWindow', 'get').mockReturnValue({
    location: { replace },
  } as unknown as Window);
  const target = openSilentAuthorizationWindow(window, false);
  const controller = new AbortController();
  controller.abort(new Error('disposed'));
  await expect(
    target.receive(new URL('https://identity.example/auth'), controller.signal),
  ).rejects.toThrow('disposed');
  expect(replace).not.toHaveBeenCalled();
  target.close();
  expect(document.querySelector('iframe')).toBeNull();
});

it('closing a frame rejects a pending receive and removes it', async () => {
  const target = openSilentAuthorizationWindow(window, false);
  const pending = target.receive(new URL('about:blank'), new AbortController().signal);
  target.close();
  await expect(pending).rejects.toMatchObject({ code: 'aborted' });
  expect(document.querySelector('iframe')).toBeNull();
});

it.each(['opener', 'parent'] as const)(
  'posts the callback only to the same-origin %s and strips the query first',
  (recipient) => {
    const postMessage = vi.fn();
    const replaceState = vi.fn();
    const browser = {
      location: {
        href: 'https://app.example/callback?code=secret&state=state',
        pathname: '/callback',
        origin: 'https://app.example',
      },
      history: { replaceState },
      opener: recipient === 'opener' ? { postMessage } : null,
      parent: { postMessage },
    };
    expect(completeSilentTokenCallback(browser as unknown as Window)).toBe(true);
    expect(CALLBACK_MESSAGE).toBe('baukit-silent-token-callback');
    expect(replaceState).toHaveBeenCalledWith(null, '', '/callback');
    expect(postMessage).toHaveBeenCalledWith(
      { type: CALLBACK_MESSAGE, url: browser.location.href },
      'https://app.example',
    );
    expect(replaceState.mock.invocationCallOrder[0]).toBeLessThan(
      postMessage.mock.invocationCallOrder[0] ?? 0,
    );
  },
);

it('cleans a standalone callback without posting a message', () => {
  const browser = {
    location: window.location,
    history: { replaceState: vi.fn() },
    opener: null,
    parent: {},
  };
  browser.parent = browser;
  expect(completeSilentTokenCallback(browser as unknown as Window)).toBe(false);
  expect(browser.history.replaceState).toHaveBeenCalled();
});

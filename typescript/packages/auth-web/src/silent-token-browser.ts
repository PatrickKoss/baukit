import type { SilentAuthorizationWindow } from './silent-token-broker.js';
import { SilentTokenError } from './silent-token-errors.js';

const CALLBACK_MESSAGE = 'baukit-silent-token-callback';

/** Run on a dedicated same-origin callback page with no application initialization. */
export function completeSilentTokenCallback(browser: Window = window): boolean {
  const callbackUrl = browser.location.href;
  browser.history.replaceState(null, '', browser.location.pathname);
  const opener: Window | null = browser.opener as Window | null;
  const recipient = opener ?? (browser.parent !== browser ? browser.parent : null);
  if (!recipient) return false;
  recipient.postMessage({ type: CALLBACK_MESSAGE, url: callbackUrl }, browser.location.origin);
  return true;
}

export function openSilentAuthorizationWindow(
  browser: Window,
  interactive: boolean,
): SilentAuthorizationWindow {
  const frame = interactive ? undefined : browser.document.createElement('iframe');
  if (frame) {
    frame.hidden = true;
    frame.title = 'Authorization';
    browser.document.body.append(frame);
  }
  const popup = interactive
    ? browser.open('about:blank', '_blank', 'popup,width=520,height=720')
    : null;
  const target = frame?.contentWindow ?? popup;
  if (!target) {
    frame?.remove();
    throw new SilentTokenError('popup_blocked');
  }
  let cleanup: () => void = () => undefined;
  let cancel: () => void = () => undefined;
  return {
    receive: (url, signal) =>
      new Promise<string>((resolve, reject) => {
        const finish = (value?: string) => {
          cleanup();
          if (value !== undefined) resolve(value);
          else
            reject(
              signal.reason instanceof Error ? signal.reason : new SilentTokenError('aborted'),
            );
        };
        const abort = () => {
          finish();
        };
        const message = (event: MessageEvent<unknown>) => {
          if (
            event.origin !== browser.location.origin ||
            event.source !== target ||
            typeof event.data !== 'object' ||
            event.data === null ||
            !('type' in event.data) ||
            event.data.type !== CALLBACK_MESSAGE ||
            !('url' in event.data) ||
            typeof event.data.url !== 'string'
          )
            return;
          finish(event.data.url);
        };
        cleanup = () => {
          browser.removeEventListener('message', message);
          signal.removeEventListener('abort', abort);
        };
        cancel = abort;
        browser.addEventListener('message', message);
        signal.addEventListener('abort', abort, { once: true });
        if (signal.aborted) {
          abort();
          return;
        }
        try {
          target.location.replace(url.toString());
        } catch (cause) {
          cleanup();
          reject(cause instanceof Error ? cause : new SilentTokenError('aborted'));
        }
      }),
    close: () => {
      cancel();
      cleanup();
      frame?.remove();
      popup?.close();
    },
  };
}

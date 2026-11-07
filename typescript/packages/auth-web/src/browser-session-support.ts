import { isRecord } from '@baukit/type-guards';

import { browserOidcEnvironment, type OidcClientEnvironment } from './oidc-client.js';
import type { SessionExpiredReason } from './session-events.js';

/** The part of the Web Locks API the session client needs. */
export interface SessionLocks {
  request<T>(name: string, callback: () => Promise<T>): Promise<T>;
}

/** The part of `BroadcastChannel` the session client needs. */
export interface SessionChannel {
  postMessage(message: unknown): void;
  subscribe(listener: (message: unknown) => void): void;
  close(): void;
}

export interface BrowserSessionEnvironment extends OidcClientEnvironment {
  /** Without locks, tabs may refresh concurrently and the backend grace window absorbs it. */
  readonly locks?: SessionLocks;
  readonly openChannel?: (name: string) => SessionChannel;
}

/** What the backend returns from `exchange` and `refresh`. */
export interface IssuedSession {
  readonly accessToken: string;
  /** Epoch milliseconds. */
  readonly expiresAt: number;
  readonly subject: string;
}

/** Sign-in state kept in localStorage beside a session counter. It holds no token. */
export interface AccountHint {
  readonly subject: string;
  /** Epoch milliseconds. */
  readonly signedInAt: number;
}

export type SignedOutCause = SessionExpiredReason | 'cleared';

export type SessionMessage =
  | ({ readonly type: 'session' } & IssuedSession)
  | { readonly type: 'signed-out'; readonly cause: SignedOutCause };

const SIGNED_OUT_CAUSES: readonly string[] = [
  'refresh_rejected',
  'signed_out',
  'profile_erased',
  'cleared',
];

function isSignedOutCause(value: string): value is SignedOutCause {
  return SIGNED_OUT_CAUSES.includes(value);
}

export function isIssuedSession(value: unknown): value is IssuedSession {
  return (
    isRecord(value) &&
    typeof value['accessToken'] === 'string' &&
    value['accessToken'].length > 0 &&
    typeof value['expiresAt'] === 'number' &&
    Number.isFinite(value['expiresAt']) &&
    typeof value['subject'] === 'string' &&
    value['subject'].length > 0
  );
}

export function readSessionMessage(value: unknown): SessionMessage | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  if (value['type'] === 'session' && isIssuedSession(value)) {
    return {
      type: 'session',
      accessToken: value.accessToken,
      expiresAt: value.expiresAt,
      subject: value.subject,
    };
  }
  const cause = value['cause'];
  if (value['type'] === 'signed-out' && typeof cause === 'string') {
    return isSignedOutCause(cause) ? { type: 'signed-out', cause } : undefined;
  }
  return undefined;
}

export function readAccountHint(raw: string | null): AccountHint | undefined {
  if (raw === null) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(raw);
    if (
      isRecord(value) &&
      typeof value['subject'] === 'string' &&
      value['subject'].length > 0 &&
      typeof value['signedInAt'] === 'number' &&
      Number.isFinite(value['signedInAt'])
    ) {
      return { subject: value['subject'], signedInAt: value['signedInAt'] };
    }
  } catch {
    return undefined;
  }
  return undefined;
}

/** Accepts only http(s) navigation targets from the logout response. */
export function readEndSessionUrl(value: unknown): string | undefined {
  if (!isRecord(value) || typeof value['endSessionUrl'] !== 'string') {
    return undefined;
  }
  try {
    const url = new URL(value['endSessionUrl']);
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.toString() : undefined;
  } catch {
    return undefined;
  }
}

export function browserSessionEnvironment(): BrowserSessionEnvironment {
  const base = browserOidcEnvironment();
  const lockManager = typeof navigator === 'undefined' ? undefined : navigator.locks;
  return {
    ...base,
    ...(lockManager === undefined
      ? {}
      : {
          locks: {
            request: <T>(name: string, callback: () => Promise<T>) =>
              lockManager.request(name, callback),
          },
        }),
    ...(typeof BroadcastChannel === 'function' ? { openChannel: openBroadcastChannel } : {}),
  };
}

function openBroadcastChannel(name: string): SessionChannel {
  const channel = new BroadcastChannel(name);
  return {
    postMessage: (message) => {
      channel.postMessage(message);
    },
    subscribe: (listener) => {
      channel.addEventListener('message', (event: MessageEvent<unknown>) => {
        listener(event.data);
      });
    },
    close: () => {
      channel.close();
    },
  };
}

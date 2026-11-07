import { vi } from 'vitest';

import type {
  BrowserSessionEnvironment,
  SessionChannel,
  SessionLocks,
} from '../browser-session-support.js';

export class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  public get length(): number {
    return this.values.size;
  }

  public clear(): void {
    this.values.clear();
  }

  public getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  public key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }

  public removeItem(key: string): void {
    this.values.delete(key);
  }

  public setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

/** Grants each named lock to one holder at a time, in request order, like Web Locks. */
export function fakeLocks(): SessionLocks & { readonly requests: string[] } {
  const tails = new Map<string, Promise<unknown>>();
  const requests: string[] = [];
  return {
    requests,
    request: <T>(name: string, callback: () => Promise<T>): Promise<T> => {
      requests.push(name);
      const run = (tails.get(name) ?? Promise.resolve()).then(callback);
      tails.set(
        name,
        run.then(
          () => undefined,
          () => undefined,
        ),
      );
      return run;
    },
  };
}

/** Delivers each message asynchronously to every other channel with the same name. */
export class ChannelHub {
  public readonly posted: unknown[] = [];
  private readonly channels = new Set<{ name: string; listeners: ((m: unknown) => void)[] }>();

  public open = (name: string): SessionChannel => {
    const entry = { name, listeners: [] as ((m: unknown) => void)[] };
    this.channels.add(entry);
    return {
      postMessage: (message) => {
        this.posted.push(message);
        const copy: unknown = structuredClone(message);
        for (const other of this.channels) {
          if (other !== entry && other.name === name) {
            queueMicrotask(() => {
              for (const listener of other.listeners) listener(copy);
            });
          }
        }
      },
      subscribe: (listener) => {
        entry.listeners.push(listener);
      },
      close: () => {
        this.channels.delete(entry);
      },
    };
  };
}

export interface Tab {
  readonly environment: BrowserSessionEnvironment;
  readonly sessionStorage: MemoryStorage;
  readonly navigations: string[];
  setCurrentUrl(url: string): void;
}

export interface Browser {
  readonly localStorage: MemoryStorage;
  readonly locks: ReturnType<typeof fakeLocks>;
  readonly hub: ChannelHub;
  readonly fetch: ReturnType<typeof vi.fn<typeof globalThis.fetch>>;
  now(): number;
  setNow(value: number): void;
  tab(options?: { readonly coordinated?: boolean }): Tab;
}

export function fakeBrowser(fetch: typeof globalThis.fetch): Browser {
  const localStorage = new MemoryStorage();
  const locks = fakeLocks();
  const hub = new ChannelHub();
  const fetchMock = vi.fn<typeof globalThis.fetch>(fetch);
  let now = 1_000_000;
  return {
    localStorage,
    locks,
    hub,
    fetch: fetchMock,
    now: () => now,
    setNow: (value) => {
      now = value;
    },
    tab: ({ coordinated = true } = {}) => {
      const sessionStorage = new MemoryStorage();
      const navigations: string[] = [];
      let currentUrl = 'https://app.example.test/';
      return {
        sessionStorage,
        navigations,
        setCurrentUrl: (url) => {
          currentUrl = url;
        },
        environment: {
          fetch: fetchMock,
          crypto: globalThis.crypto,
          localStorage,
          sessionStorage,
          currentUrl: () => currentUrl,
          navigate: (url) => navigations.push(url),
          replaceUrl: () => undefined,
          now: () => now,
          ...(coordinated ? { locks, openChannel: hub.open } : {}),
        },
      };
    },
  };
}

export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

export function problem(status: number, code: string): Response {
  return jsonResponse({ code, message: 'problem' }, status);
}

export function discoveryResponse(): Response {
  return jsonResponse({
    issuer: 'https://identity.example.test/realm',
    authorization_endpoint: 'https://identity.example.test/realm/authorize',
    token_endpoint: 'https://identity.example.test/realm/token',
  });
}

export function requestUrl(input: RequestInfo | URL): string {
  return input instanceof Request ? input.url : input.toString();
}

export function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) {
    throw new Error('Expected test value to be present.');
  }
  return value;
}

export function readLoginTransaction(
  storage: Storage,
  key: string,
): { state: string; verifier: string } {
  const value: unknown = JSON.parse(required(storage.getItem(key)));
  if (
    typeof value !== 'object' ||
    value === null ||
    !('state' in value) ||
    !('verifier' in value) ||
    typeof value.state !== 'string' ||
    typeof value.verifier !== 'string'
  ) {
    throw new Error('Expected a login transaction.');
  }
  return { state: value.state, verifier: value.verifier };
}

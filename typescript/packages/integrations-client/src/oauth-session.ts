export interface OAuthInFlightSession {
  readonly stateNonce: string;
  readonly returnUrl: string;
  readonly createdAt: number;
}

export interface OAuthSessionStorage {
  load(): Promise<OAuthInFlightSession | null>;
  save(session: OAuthInFlightSession): Promise<void>;
  clear(): Promise<void>;
}

export interface OAuthClock {
  now(): number;
  setTimeout(callback: () => void, timeoutMs: number): unknown;
  clearTimeout(handle: unknown): void;
}

export type OAuthSessionResult =
  { readonly type: 'success'; readonly returnUrl: string } | { readonly type: 'cancelled' };

export interface BrowserPopupSession {
  readonly result: Promise<OAuthSessionResult>;
  navigate(authorizationUrl: string): void;
  close(): void;
}

export interface BrowserPopupOpener {
  openPlaceholder(input: {
    readonly placeholderUrl: string;
    readonly returnUrl: string;
    readonly windowName: string;
  }): BrowserPopupSession | null;
}

export interface NativeAuthSessionRunner {
  run(input: {
    readonly authorizationUrl: string;
    readonly returnUrl: string;
  }): Promise<OAuthSessionResult>;
  cancel(): void;
}

export interface OAuthRedirectHandler {
  authorize(input: {
    readonly authorizationUrl: string;
    readonly returnUrl: string;
  }): Promise<OAuthSessionResult>;
}

export interface OAuthReturnUrlPolicy {
  readonly origin: string;
  readonly paths: readonly string[];
}

export type OAuthReturnUrlValidator = (returnUrl: string) => boolean;

export type OAuthSessionOutcome =
  | {
      readonly type: 'success';
      readonly callbackParams: Readonly<Record<string, string>>;
    }
  | { readonly type: 'cancelled' }
  | { readonly type: 'timed_out' }
  | { readonly type: 'invalid_return' }
  | { readonly type: 'storage_mismatch' };

export interface OAuthSessionCoordinatorOptions {
  readonly storage: OAuthSessionStorage;
  readonly clock: OAuthClock;
  readonly timeoutMs: number;
  readonly createStateNonce: () => string;
  readonly validateReturnUrl: OAuthReturnUrlValidator;
  readonly browser?: BrowserPopupOpener;
  readonly native?: NativeAuthSessionRunner;
  readonly redirect?: OAuthRedirectHandler;
  readonly placeholderUrl?: string;
  readonly windowName?: string;
}

export interface OAuthAuthorizationRequest {
  readonly platform: 'web' | 'native';
  readonly returnUrl: string;
  readonly createAuthorizationUrl: (input: {
    readonly stateNonce: string;
    readonly returnUrl: string;
  }) => Promise<string>;
}

const DEFAULT_PLACEHOLDER_URL = 'about:blank';
const DEFAULT_WINDOW_NAME = 'baukit-integration-oauth';
const TIMED_OUT = Symbol('timed_out');

export class OAuthSessionCoordinator {
  readonly #options: OAuthSessionCoordinatorOptions;

  public constructor(options: OAuthSessionCoordinatorOptions) {
    if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
      throw new TypeError('OAuth timeout must be a positive finite number.');
    }
    this.#options = options;
  }

  public async authorize(request: OAuthAuthorizationRequest): Promise<OAuthSessionOutcome> {
    if (!this.#options.validateReturnUrl(request.returnUrl)) return { type: 'invalid_return' };

    const stateNonce = this.#options.createStateNonce();
    if (stateNonce.length === 0) throw new TypeError('OAuth state nonce must not be empty.');

    const popup = request.platform === 'web' ? this.openBrowserPopup(request.returnUrl) : null;
    const operation = this.performAuthorization(request, stateNonce, popup);
    const result = await this.withTimeout(operation, () => {
      popup?.close();
      if (request.platform === 'native') this.#options.native?.cancel();
    });

    if (result === TIMED_OUT) {
      await this.clearIfCurrent(stateNonce);
      return { type: 'timed_out' };
    }
    return result;
  }

  public async handleRedirect(returnUrl: string): Promise<OAuthSessionOutcome> {
    return this.completeReturn(returnUrl);
  }

  public async discardStaleSession(): Promise<boolean> {
    const stored = await this.#options.storage.load();
    if (stored === null || !this.isStale(stored)) return false;
    await this.#options.storage.clear();
    return true;
  }

  private openBrowserPopup(returnUrl: string): BrowserPopupSession | null {
    const browser = this.#options.browser;
    if (browser === undefined) return null;
    return browser.openPlaceholder({
      placeholderUrl: this.#options.placeholderUrl ?? DEFAULT_PLACEHOLDER_URL,
      returnUrl,
      windowName: this.#options.windowName ?? DEFAULT_WINDOW_NAME,
    });
  }

  private async performAuthorization(
    request: OAuthAuthorizationRequest,
    stateNonce: string,
    popup: BrowserPopupSession | null,
  ): Promise<OAuthSessionOutcome> {
    await this.discardStaleSession();
    await this.#options.storage.save({
      stateNonce,
      returnUrl: request.returnUrl,
      createdAt: this.#options.clock.now(),
    });
    const authorizationUrl = await request.createAuthorizationUrl({
      stateNonce,
      returnUrl: request.returnUrl,
    });

    let sessionResult: OAuthSessionResult;
    if (request.platform === 'native') {
      const native = this.#options.native;
      if (native === undefined) throw new TypeError('Native OAuth runner is required.');
      sessionResult = await native.run({ authorizationUrl, returnUrl: request.returnUrl });
    } else if (popup !== null) {
      popup.navigate(authorizationUrl);
      sessionResult = await popup.result;
    } else {
      const redirect = this.#options.redirect;
      if (redirect === undefined) throw new TypeError('Browser OAuth handler is required.');
      sessionResult = await redirect.authorize({
        authorizationUrl,
        returnUrl: request.returnUrl,
      });
    }

    if (sessionResult.type === 'cancelled') {
      await this.clearIfCurrent(stateNonce);
      return { type: 'cancelled' };
    }
    return this.completeReturn(sessionResult.returnUrl, stateNonce);
  }

  private async completeReturn(
    returnUrl: string,
    expectedStateNonce?: string,
  ): Promise<OAuthSessionOutcome> {
    if (!this.#options.validateReturnUrl(returnUrl)) return { type: 'invalid_return' };
    const callbackParams = parseCallbackParams(returnUrl);
    if (callbackParams === null) return { type: 'invalid_return' };

    const stored = await this.#options.storage.load();
    if (stored === null) return { type: 'storage_mismatch' };
    if (this.isStale(stored)) {
      await this.#options.storage.clear();
      return { type: 'timed_out' };
    }
    if (
      (expectedStateNonce !== undefined && stored.stateNonce !== expectedStateNonce) ||
      callbackParams['state'] !== stored.stateNonce
    ) {
      return { type: 'storage_mismatch' };
    }
    if (!sameReturnEndpoint(stored.returnUrl, returnUrl)) return { type: 'invalid_return' };

    await this.#options.storage.clear();
    return { type: 'success', callbackParams };
  }

  private isStale(session: OAuthInFlightSession): boolean {
    const elapsed = this.#options.clock.now() - session.createdAt;
    return elapsed < 0 || elapsed >= this.#options.timeoutMs;
  }

  private async clearIfCurrent(stateNonce: string): Promise<void> {
    const stored = await this.#options.storage.load();
    if (stored?.stateNonce === stateNonce) await this.#options.storage.clear();
  }

  private async withTimeout<T>(
    operation: Promise<T>,
    onTimeout: () => void,
  ): Promise<T | typeof TIMED_OUT> {
    let timer: unknown;
    const timeout = new Promise<typeof TIMED_OUT>((resolve) => {
      timer = this.#options.clock.setTimeout(() => {
        try {
          onTimeout();
        } finally {
          resolve(TIMED_OUT);
        }
      }, this.#options.timeoutMs);
    });
    try {
      return await Promise.race([operation, timeout]);
    } finally {
      this.#options.clock.clearTimeout(timer);
    }
  }
}

export function createReturnUrlValidator(policy: OAuthReturnUrlPolicy): OAuthReturnUrlValidator {
  const origin = parseOrigin(policy.origin);
  const paths = new Set(policy.paths.map(normalizeAllowedPath));
  if (paths.size === 0) throw new TypeError('At least one OAuth return path is required.');
  return (returnUrl) => {
    const parsed = parseUrl(returnUrl);
    return (
      parsed !== null &&
      parsed.username === '' &&
      parsed.password === '' &&
      exactOrigin(parsed) === origin &&
      paths.has(parsed.pathname)
    );
  };
}

function parseOrigin(origin: string): string {
  const parsed = parseUrl(origin);
  if (
    parsed?.username !== '' ||
    parsed.password !== '' ||
    (parsed.pathname !== '' && parsed.pathname !== '/') ||
    parsed.search !== '' ||
    parsed.hash !== ''
  ) {
    throw new TypeError('OAuth return origin must contain only a scheme and authority.');
  }
  return exactOrigin(parsed);
}

function normalizeAllowedPath(path: string): string {
  if (!path.startsWith('/') || path.includes('?') || path.includes('#')) {
    throw new TypeError('OAuth return paths must be absolute paths without a query or fragment.');
  }
  const parsed = new URL(path, 'https://baukit.invalid');
  if (parsed.pathname !== path) throw new TypeError('OAuth return paths must be normalized.');
  return path;
}

function parseCallbackParams(returnUrl: string): Readonly<Record<string, string>> | null {
  const parsed = parseUrl(returnUrl);
  if (parsed === null) return null;
  const values: Record<string, string> = {};
  for (const [key, value] of parsed.searchParams) {
    if (Object.hasOwn(values, key)) return null;
    values[key] = value;
  }
  return Object.freeze(values);
}

function sameReturnEndpoint(expected: string, actual: string): boolean {
  const expectedUrl = parseUrl(expected);
  const actualUrl = parseUrl(actual);
  return (
    expectedUrl !== null &&
    actualUrl !== null &&
    exactOrigin(expectedUrl) === exactOrigin(actualUrl) &&
    expectedUrl.pathname === actualUrl.pathname
  );
}

function exactOrigin(url: URL): string {
  return url.origin === 'null' ? `${url.protocol}//${url.host}` : url.origin;
}

function parseUrl(value: string): URL | null {
  try {
    return new URL(value);
  } catch {
    return null;
  }
}

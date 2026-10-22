import { createLocalJWKSet, jwtVerify, type JSONWebKeySet } from 'jose';
import {
  base64UrlEncode,
  buildAuthorizationUrl,
  normalizeIssuer,
  normalizeScopes,
  randomUrlSafe,
} from './oidc-helpers.js';
import { openSilentAuthorizationWindow } from './silent-token-browser.js';
import { SilentTokenError } from './silent-token-errors.js';

export { SilentTokenError, type SilentTokenErrorCode } from './silent-token-errors.js';

export interface SilentTokenProduct {
  readonly id: string;
  readonly issuer: string;
  readonly clientId: string;
  readonly audience: string;
  readonly redirectUri: string;
  /** Defaults to `openid`, which is always included. */
  readonly scopes?: readonly string[];
  /** Provider options, such as an identity provider hint. Cannot override OIDC parameters. */
  readonly authorizationParams?: Readonly<Record<string, string>>;
}

export interface SilentTokenBrokerOptions {
  /** A signed access token claim that binds each product account to the active session. */
  readonly accountBinding: { readonly claim: string; readonly value: string };
}

export type SilentLoginReason =
  'login_required' | 'interaction_required' | 'interaction_cancelled' | 'timeout';

export type SilentTokenResult =
  | {
      readonly status: 'ok';
      readonly productId: string;
      readonly accessToken: string;
      readonly expiresAt: number;
    }
  | {
      readonly status: 'login_required';
      readonly productId: string;
      readonly reason: SilentLoginReason;
    }
  | { readonly status: 'consent_required'; readonly productId: string }
  | { readonly status: 'account_mismatch'; readonly productId: string }
  | { readonly status: 'error'; readonly productId: string; readonly error: SilentTokenError };

type AuthorizationFailure = SilentLoginReason | 'consent_required' | 'account_mismatch';

class AuthorizationError extends Error {
  public constructor(public readonly code: AuthorizationFailure) {
    super(code);
  }
}

export interface SilentAuthorizationWindow {
  /** True when the interactive popup has closed. Omit if closure cannot be observed. */
  readonly closed?: boolean;
  receive(url: URL, signal: AbortSignal): Promise<string>;
  close(): void;
}

export interface SilentTokenEnvironment {
  readonly fetch: typeof globalThis.fetch;
  readonly crypto: Crypto;
  readonly now: () => number;
  /** Must open interactive windows synchronously, before returning. */
  readonly open: (interactive: boolean) => SilentAuthorizationWindow;
}

interface Metadata {
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly jwksUri: string;
}
interface CachedToken {
  readonly result: Extract<SilentTokenResult, { status: 'ok' }>;
  readonly renewAt: number;
}
interface TokenResponse {
  readonly accessToken: string;
  readonly idToken: string;
  readonly responseExpiresAt: number;
}

const SILENT_TIMEOUT_MS = 15_000;
const INTERACTIVE_TIMEOUT_MS = 180_000;
const POPUP_POLL_INTERVAL_MS = 250;
const EXPIRY_LEEWAY_MS = 30_000;
const RESERVED_PARAMETERS = new Set([
  'client_id',
  'redirect_uri',
  'response_type',
  'response_mode',
  'scope',
  'state',
  'nonce',
  'code_challenge',
  'code_challenge_method',
  'prompt',
  'audience',
  'resource',
]);

/** One instance belongs to one active session. Dispose it before changing identity. */
export class SilentTokenBroker {
  private readonly products = new Map<string, SilentTokenProduct>();
  private readonly cache = new Map<string, CachedToken>();
  private readonly pending = new Map<string, Promise<SilentTokenResult>>();
  private readonly failures = new Map<string, SilentTokenResult>();
  private readonly controllers = new Set<AbortController>();
  private readonly accountBinding: SilentTokenBrokerOptions['accountBinding'];
  private disposed = false;

  public constructor(
    products: readonly SilentTokenProduct[],
    options: SilentTokenBrokerOptions,
    private readonly environment: SilentTokenEnvironment = browserEnvironment(),
  ) {
    if (!options.accountBinding.claim.trim() || !options.accountBinding.value.trim()) {
      throw new SilentTokenError('invalid_account_binding');
    }
    this.accountBinding = { ...options.accountBinding };
    for (const product of products) {
      if (
        this.products.has(product.id) ||
        !product.id.trim() ||
        !product.clientId.trim() ||
        !product.audience.trim()
      ) {
        throw new SilentTokenError('invalid_product');
      }
      trustedUrl(product.issuer);
      const redirectUri = trustedUrl(product.redirectUri).toString();
      const authorizationParams = { ...product.authorizationParams };
      for (const [key, value] of Object.entries(authorizationParams)) {
        if (!key.trim() || RESERVED_PARAMETERS.has(key) || !value.trim()) {
          throw new SilentTokenError('invalid_product');
        }
      }
      this.products.set(product.id, {
        ...product,
        issuer: normalizeIssuer(product.issuer),
        clientId: product.clientId.trim(),
        audience: product.audience.trim(),
        redirectUri,
        scopes: normalizeScopes(product.scopes ?? ['openid']),
        authorizationParams,
      });
    }
  }

  public getToken(productId: string, audience?: string): Promise<SilentTokenResult> {
    const product = this.product(productId, audience);
    const cached = this.cache.get(product.id);
    if (cached && cached.renewAt > this.environment.now()) return Promise.resolve(cached.result);
    this.cache.delete(product.id);
    const pending = this.pending.get(product.id);
    if (pending) return pending;
    const failure = this.failures.get(product.id);
    return failure ? Promise.resolve(failure) : this.start(product, false);
  }

  /** Call directly from a user gesture. Opens a popup before any asynchronous work. */
  public login(productId: string): Promise<SilentTokenResult> {
    const product = this.product(productId);
    return this.pending.get(product.id) ?? this.start(product, true);
  }

  /** Discards a token after an API rejection. Remembered failures remain latched. */
  public invalidate(productId: string): void {
    this.cache.delete(this.product(productId).id);
  }

  /** Allows the next getToken() to retry a remembered failure. Does not start work. */
  public resetFailure(productId: string): void {
    this.failures.delete(this.product(productId).id);
  }

  public dispose(): void {
    this.disposed = true;
    for (const controller of this.controllers) controller.abort(new SilentTokenError('disposed'));
    this.controllers.clear();
    this.cache.clear();
    this.failures.clear();
    this.pending.clear();
    this.products.clear();
  }

  private product(id: string, audience?: string): SilentTokenProduct {
    if (this.disposed) throw new SilentTokenError('disposed');
    const product = this.products.get(id);
    if (!product || (audience !== undefined && product.audience !== audience)) {
      throw new SilentTokenError('unknown_product_or_audience');
    }
    return product;
  }

  private start(product: SilentTokenProduct, interactive: boolean): Promise<SilentTokenResult> {
    this.cache.delete(product.id);
    const pending = this.authorize(product, interactive)
      .then((result) => {
        if (this.disposed) return failureResult(product.id, new SilentTokenError('disposed'));
        if (result.status === 'ok') {
          const lifetime = result.expiresAt - this.environment.now();
          this.cache.set(product.id, {
            result,
            renewAt: result.expiresAt - Math.min(EXPIRY_LEEWAY_MS, lifetime / 2),
          });
          this.failures.delete(product.id);
        } else {
          const previous = this.failures.get(product.id);
          if (!previous || result.status !== 'error' || result.error.code !== 'popup_blocked')
            this.failures.set(product.id, result);
        }
        return result;
      })
      .finally(() => {
        this.pending.delete(product.id);
      });
    this.pending.set(product.id, pending);
    return pending;
  }

  private async authorize(
    product: SilentTokenProduct,
    interactive: boolean,
  ): Promise<SilentTokenResult> {
    const controller = new AbortController();
    this.controllers.add(controller);
    const timer = setTimeout(
      () => {
        controller.abort(new AuthorizationError('timeout'));
      },
      interactive ? INTERACTIVE_TIMEOUT_MS : SILENT_TIMEOUT_MS,
    );
    let target: SilentAuthorizationWindow | undefined;
    let popupPoll: ReturnType<typeof setInterval> | undefined;
    try {
      target = this.environment.open(interactive);
      if (interactive) {
        popupPoll = setInterval(() => {
          if (target?.closed) controller.abort(new AuthorizationError('interaction_cancelled'));
        }, POPUP_POLL_INTERVAL_MS);
      }
      return await abortable(
        this.request(product, interactive, target, controller.signal),
        controller.signal,
      );
    } catch (cause) {
      return failureResult(
        product.id,
        controller.signal.aborted ? controller.signal.reason : cause,
      );
    } finally {
      clearTimeout(timer);
      clearInterval(popupPoll);
      this.controllers.delete(controller);
      target?.close();
    }
  }

  private async request(
    product: SilentTokenProduct,
    interactive: boolean,
    target: SilentAuthorizationWindow,
    signal: AbortSignal,
  ): Promise<SilentTokenResult> {
    const metadata = await this.metadata(product, signal);
    const state = randomUrlSafe(this.environment.crypto, 32);
    const nonce = randomUrlSafe(this.environment.crypto, 32);
    const verifier = randomUrlSafe(this.environment.crypto, 64);
    const digest = await this.environment.crypto.subtle.digest(
      'SHA-256',
      new TextEncoder().encode(verifier),
    );
    signal.throwIfAborted();
    const url = buildAuthorizationUrl(product, metadata.authorizationEndpoint, {
      state,
      challenge: base64UrlEncode(new Uint8Array(digest)),
    });
    url.searchParams.set('response_mode', 'query');
    url.searchParams.set('nonce', nonce);
    if (!interactive) url.searchParams.set('prompt', 'none');
    for (const [key, value] of Object.entries(product.authorizationParams ?? {}))
      url.searchParams.set(key, value);
    const callback = await target.receive(url, signal);
    signal.throwIfAborted();
    const code = callbackCode(callback, product, state);
    return this.exchange(product, metadata, { code, verifier, nonce }, signal);
  }

  private async exchange(
    product: SilentTokenProduct,
    metadata: Metadata,
    transaction: { code: string; verifier: string; nonce: string },
    signal: AbortSignal,
  ): Promise<Extract<SilentTokenResult, { status: 'ok' }>> {
    const { value, receivedAt } = await this.json(metadata.tokenEndpoint, signal, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: product.clientId,
        redirect_uri: product.redirectUri,
        code: transaction.code,
        code_verifier: transaction.verifier,
      }),
    });
    const tokens = validateTokenResponse(value, receivedAt);
    const signedExpiresAt = await this.verifyTokenClaims(
      product,
      metadata.jwksUri,
      tokens,
      transaction.nonce,
      signal,
    );
    signal.throwIfAborted();
    const expiresAt = Math.min(tokens.responseExpiresAt, signedExpiresAt);
    if (expiresAt <= this.environment.now()) throw new SilentTokenError('invalid_tokens');
    return { status: 'ok', productId: product.id, accessToken: tokens.accessToken, expiresAt };
  }

  private async verifyTokenClaims(
    product: SilentTokenProduct,
    jwksUri: string,
    tokens: TokenResponse,
    nonce: string,
    signal: AbortSignal,
  ): Promise<number> {
    const { value: jwks } = await this.json(jwksUri, signal);
    const keys = createLocalJWKSet(jwks as unknown as JSONWebKeySet);
    const options = {
      issuer: product.issuer,
      algorithms: ['RS256'],
      currentDate: new Date(this.environment.now()),
      requiredClaims: ['exp', 'iat', 'sub'],
    };
    const { payload: id } = await jwtVerify(tokens.idToken, keys, {
      ...options,
      audience: product.clientId,
    });
    if (id['nonce'] !== nonce) throw new SilentTokenError('invalid_nonce');
    if (
      (Array.isArray(id.aud) && id.aud.length > 1 && id['azp'] !== product.clientId) ||
      (id['azp'] !== undefined && id['azp'] !== product.clientId)
    )
      throw new SilentTokenError('invalid_client');
    const { payload: access } = await jwtVerify(tokens.accessToken, keys, {
      ...options,
      audience: product.audience,
    });
    if (
      !id.sub ||
      access.sub !== id.sub ||
      access['azp'] !== product.clientId ||
      access[this.accountBinding.claim] !== this.accountBinding.value
    )
      throw new AuthorizationError('account_mismatch');
    return (access.exp ?? 0) * 1000;
  }

  private async metadata(product: SilentTokenProduct, signal: AbortSignal): Promise<Metadata> {
    const { value: raw } = await this.json(
      `${product.issuer}/.well-known/openid-configuration`,
      signal,
    );
    if (raw['issuer'] !== product.issuer) throw new SilentTokenError('invalid_issuer');
    const endpoint = (field: string): string => {
      const value = raw[field];
      if (
        typeof value !== 'string' ||
        trustedUrl(value).origin !== new URL(product.issuer).origin
      ) {
        throw new SilentTokenError('invalid_metadata');
      }
      return value;
    };
    return {
      authorizationEndpoint: endpoint('authorization_endpoint'),
      tokenEndpoint: endpoint('token_endpoint'),
      jwksUri: endpoint('jwks_uri'),
    };
  }

  private async json(
    url: string,
    signal: AbortSignal,
    init: RequestInit = {},
  ): Promise<{ value: Record<string, unknown>; receivedAt: number }> {
    signal.throwIfAborted();
    const response = await this.environment.fetch(url, {
      ...init,
      signal,
      credentials: 'omit',
      cache: 'no-store',
      redirect: 'error',
    });
    const receivedAt = this.environment.now();
    signal.throwIfAborted();
    if (!response.ok) throw new SilentTokenError('http_error');
    const value: unknown = await response.json();
    signal.throwIfAborted();
    if (typeof value !== 'object' || value === null || Array.isArray(value))
      throw new SilentTokenError('invalid_response');
    return { value: value as Record<string, unknown>, receivedAt };
  }
}

function validateTokenResponse(tokens: Record<string, unknown>, receivedAt: number): TokenResponse {
  if (
    typeof tokens['access_token'] !== 'string' ||
    !tokens['access_token'] ||
    typeof tokens['id_token'] !== 'string' ||
    !tokens['id_token'] ||
    typeof tokens['expires_in'] !== 'number' ||
    !Number.isFinite(tokens['expires_in']) ||
    tokens['expires_in'] <= 0 ||
    !Number.isFinite(receivedAt + tokens['expires_in'] * 1000) ||
    tokens['token_type'] !== 'Bearer'
  ) {
    throw new SilentTokenError('invalid_tokens');
  }
  return {
    accessToken: tokens['access_token'],
    idToken: tokens['id_token'],
    responseExpiresAt: receivedAt + tokens['expires_in'] * 1000,
  };
}

function callbackCode(callback: string, product: SilentTokenProduct, state: string): string {
  const returned = new URL(callback);
  const expected = new URL(product.redirectUri);
  if (
    returned.origin !== expected.origin ||
    returned.pathname !== expected.pathname ||
    returned.hash ||
    returned.username ||
    returned.password
  ) {
    throw new SilentTokenError('invalid_callback');
  }
  for (const field of ['state', 'iss', 'code', 'error']) {
    if (returned.searchParams.getAll(field).length > 1)
      throw new SilentTokenError('invalid_callback');
  }
  if (returned.searchParams.get('state') !== state) throw new SilentTokenError('invalid_state');
  if (returned.searchParams.get('iss') !== product.issuer)
    throw new SilentTokenError('invalid_issuer');
  const error = returned.searchParams.get('error');
  if (error !== null) {
    if (returned.searchParams.has('code')) throw new SilentTokenError('invalid_callback');
    if (
      error === 'login_required' ||
      error === 'interaction_required' ||
      error === 'consent_required'
    )
      throw new AuthorizationError(error);
    throw new SilentTokenError('authorization_rejected');
  }
  const code = returned.searchParams.get('code');
  if (!code) throw new SilentTokenError('missing_code');
  return code;
}

function failureResult(
  productId: string,
  cause: unknown,
): Exclude<SilentTokenResult, { status: 'ok' }> {
  if (cause instanceof AuthorizationError) {
    if (cause.code === 'consent_required' || cause.code === 'account_mismatch')
      return { status: cause.code, productId };
    return { status: 'login_required', productId, reason: cause.code };
  }
  return {
    status: 'error',
    productId,
    error:
      cause instanceof SilentTokenError
        ? cause
        : new SilentTokenError('unavailable_or_invalid_response'),
  };
}

function trustedUrl(value: string): URL {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new SilentTokenError('invalid_url');
  }
  if (
    (url.protocol !== 'https:' &&
      !(url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))) ||
    url.username ||
    url.password ||
    url.hash ||
    url.search
  )
    throw new SilentTokenError('invalid_url');
  return url;
}

function abortable<T>(work: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const abort = () => {
      reject(signal.reason instanceof Error ? signal.reason : new SilentTokenError('aborted'));
    };
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
    work.then(resolve, reject).finally(() => {
      signal.removeEventListener('abort', abort);
    });
  });
}

function browserEnvironment(): SilentTokenEnvironment {
  if (typeof window === 'undefined') throw new SilentTokenError('unsupported_environment');
  return {
    fetch: globalThis.fetch.bind(globalThis),
    crypto: globalThis.crypto,
    now: () => Date.now(),
    open: (interactive) => openSilentAuthorizationWindow(window, interactive),
  };
}

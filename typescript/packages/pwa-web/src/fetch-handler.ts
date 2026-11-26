import {
  CACHE_STRATEGIES,
  decideCacheStrategyDecision,
  type CacheStrategy,
  type CacheStrategyOptions,
  type CacheStrategyRequest,
} from './cache-strategy.js';

export interface FetchLikeRequest extends CacheStrategyRequest {
  readonly url: string;
}

export interface FetchHandlerPorts<TRequest extends FetchLikeRequest, TResponse> {
  readonly fetch: (request: TRequest, init?: { readonly cache?: 'no-store' }) => Promise<TResponse>;
  readonly matchCache: (request: TRequest | string) => Promise<TResponse | undefined>;
  readonly putCache: (
    strategy: CacheStrategy,
    request: TRequest,
    response: TResponse,
  ) => Promise<void>;
  readonly isCacheable: (response: TResponse) => boolean;
  readonly cloneResponse: (response: TResponse) => TResponse;
}

export interface FetchHandlerOptions<
  TRequest extends FetchLikeRequest,
  TResponse,
> extends CacheStrategyOptions {
  readonly ports: FetchHandlerPorts<TRequest, TResponse>;
  readonly onRevalidateError?: (error: unknown) => void;
  /**
   * How long network-first waits before serving a cached copy, in milliseconds. The late response
   * still updates the cache. Defaults to 4,000; `false` waits for the network.
   */
  readonly networkTimeoutMs?: number | false;
}

const DEFAULT_NETWORK_TIMEOUT_MS = 4_000;
const NETWORK_TIMEOUT = Symbol('network-timeout');

function normalizeNetworkTimeout(timeoutMs: number | false | undefined): number | null {
  if (timeoutMs === false) {
    return null;
  }
  const value = timeoutMs ?? DEFAULT_NETWORK_TIMEOUT_MS;
  if (!Number.isFinite(value) || value < 0) {
    throw new RangeError('networkTimeoutMs must be a finite non-negative number');
  }
  return value;
}

async function raceTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number | null,
): Promise<T | typeof NETWORK_TIMEOUT> {
  if (timeoutMs === null) {
    return promise;
  }
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<typeof NETWORK_TIMEOUT>((resolve) => {
    timer = setTimeout(() => {
      resolve(NETWORK_TIMEOUT);
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

async function cacheFirst<TRequest extends FetchLikeRequest, TResponse>(
  request: TRequest,
  ports: FetchHandlerPorts<TRequest, TResponse>,
): Promise<TResponse> {
  const cached = await ports.matchCache(request);
  if (cached !== undefined) {
    return cached;
  }

  const response = await ports.fetch(request);
  if (ports.isCacheable(response)) {
    await ports.putCache(CACHE_STRATEGIES.cacheFirst, request, ports.cloneResponse(response));
  }
  return response;
}

async function fetchAndStore<TRequest extends FetchLikeRequest, TResponse>(
  request: TRequest,
  ports: FetchHandlerPorts<TRequest, TResponse>,
): Promise<TResponse> {
  const response = await ports.fetch(request);
  if (ports.isCacheable(response)) {
    await ports.putCache(CACHE_STRATEGIES.networkFirst, request, ports.cloneResponse(response));
  }
  return response;
}

interface NetworkFirstOptions {
  readonly timeoutMs: number | null;
  readonly onRevalidateError: ((error: unknown) => void) | undefined;
}

async function networkFirst<TRequest extends FetchLikeRequest, TResponse>(
  request: TRequest,
  ports: FetchHandlerPorts<TRequest, TResponse>,
  options: NetworkFirstOptions,
): Promise<TResponse> {
  const network = fetchAndStore(request, ports);
  let first: TResponse | typeof NETWORK_TIMEOUT;
  try {
    first = await raceTimeout(network, options.timeoutMs);
  } catch (error) {
    const cached = await ports.matchCache(request);
    if (cached !== undefined) {
      return cached;
    }
    throw error;
  }
  if (first !== NETWORK_TIMEOUT) {
    return first;
  }
  const cached = await ports.matchCache(request);
  if (cached === undefined) {
    return network;
  }
  network.catch((error: unknown) => {
    options.onRevalidateError?.(error);
  });
  return cached;
}

async function staleWhileRevalidate<TRequest extends FetchLikeRequest, TResponse>(
  request: TRequest,
  ports: FetchHandlerPorts<TRequest, TResponse>,
  onRevalidateError: ((error: unknown) => void) | undefined,
): Promise<TResponse> {
  const cached = await ports.matchCache(request);
  const revalidate = (async () => {
    const response = await ports.fetch(request);
    if (ports.isCacheable(response)) {
      await ports.putCache(
        CACHE_STRATEGIES.staleWhileRevalidate,
        request,
        ports.cloneResponse(response),
      );
    }
    return response;
  })();

  if (cached === undefined) {
    return revalidate;
  }

  revalidate.catch((error: unknown) => {
    onRevalidateError?.(error);
  });
  return cached;
}

export function createFetchHandler<TRequest extends FetchLikeRequest, TResponse>(
  options: FetchHandlerOptions<TRequest, TResponse>,
): (request: TRequest) => Promise<TResponse> {
  const { ports, onRevalidateError, networkTimeoutMs, ...strategyOptions } = options;
  const networkFirstOptions: NetworkFirstOptions = {
    timeoutMs: normalizeNetworkTimeout(networkTimeoutMs),
    onRevalidateError,
  };

  return async (request) => {
    const { strategy, navigationFallback } = decideCacheStrategyDecision(request, strategyOptions);
    try {
      switch (strategy) {
        case CACHE_STRATEGIES.networkOnly:
          return await ports.fetch(request, { cache: 'no-store' });
        case CACHE_STRATEGIES.cacheFirst:
          return await cacheFirst(request, ports);
        case CACHE_STRATEGIES.staleWhileRevalidate:
          return await staleWhileRevalidate(request, ports, onRevalidateError);
        case CACHE_STRATEGIES.networkFirst:
          return await networkFirst(request, ports, networkFirstOptions);
      }
    } catch (error) {
      if (navigationFallback !== undefined) {
        const fallback = await ports.matchCache(navigationFallback);
        if (fallback !== undefined) {
          return fallback;
        }
      }
      throw error;
    }
  };
}

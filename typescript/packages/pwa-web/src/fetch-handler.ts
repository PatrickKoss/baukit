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

async function networkFirst<TRequest extends FetchLikeRequest, TResponse>(
  request: TRequest,
  ports: FetchHandlerPorts<TRequest, TResponse>,
): Promise<TResponse> {
  try {
    const response = await ports.fetch(request);
    if (ports.isCacheable(response)) {
      await ports.putCache(CACHE_STRATEGIES.networkFirst, request, ports.cloneResponse(response));
    }
    return response;
  } catch (error) {
    const cached = await ports.matchCache(request);
    if (cached !== undefined) {
      return cached;
    }
    throw error;
  }
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
  const { ports, onRevalidateError, ...strategyOptions } = options;

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
          return await networkFirst(request, ports);
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

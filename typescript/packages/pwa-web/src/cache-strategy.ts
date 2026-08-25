export const CACHE_STRATEGIES = {
  cacheFirst: 'cache-first',
  networkFirst: 'network-first',
  networkOnly: 'network-only',
  staleWhileRevalidate: 'stale-while-revalidate',
} as const;

export type CacheStrategy = (typeof CACHE_STRATEGIES)[keyof typeof CACHE_STRATEGIES];

export interface CacheStrategyRequest {
  readonly url: string;
  readonly method?: string;
  readonly mode?: string;
  readonly destination?: string;
}

export interface CacheStrategyOptions {
  readonly appOrigin: string;
  readonly apiPathPrefixes?: readonly string[];
  readonly neverCachedPathPrefixes?: readonly string[];
  readonly navigationFallback?: string;
  readonly staticPathPrefixes?: readonly string[];
  readonly staticDestinations?: readonly string[];
  readonly staticExtensions?: readonly string[];
  readonly apiStrategy?: CacheStrategy;
  readonly staticStrategy?: CacheStrategy;
  readonly defaultStrategy?: CacheStrategy;
}

export interface CacheStrategyDecision {
  readonly strategy: CacheStrategy;
  readonly navigationFallback?: string;
}

const DEFAULT_API_PATH_PREFIXES: readonly string[] = ['/api'];

const DEFAULT_STATIC_DESTINATIONS: readonly string[] = [
  'audio',
  'font',
  'image',
  'script',
  'style',
  'video',
  'worker',
];

const DEFAULT_STATIC_EXTENSIONS: readonly string[] = [
  'avif',
  'css',
  'gif',
  'ico',
  'jpeg',
  'jpg',
  'js',
  'json',
  'mjs',
  'mp3',
  'mp4',
  'ogg',
  'otf',
  'png',
  'svg',
  'ttf',
  'webmanifest',
  'webp',
  'woff',
  'woff2',
];

function matchesPathPrefix(pathname: string, prefix: string): boolean {
  if (prefix.length === 0) {
    return false;
  }
  const normalized = prefix.endsWith('/') ? prefix.slice(0, -1) : prefix;
  return pathname === normalized || pathname.startsWith(`${normalized}/`);
}

function matchesAnyPathPrefix(pathname: string, prefixes: readonly string[]): boolean {
  return prefixes.some((prefix) => matchesPathPrefix(pathname, prefix));
}

function hasStaticExtension(pathname: string, extensions: readonly string[]): boolean {
  const lastSegment = pathname.slice(pathname.lastIndexOf('/') + 1);
  const dotIndex = lastSegment.lastIndexOf('.');
  if (dotIndex <= 0) {
    return false;
  }
  const extension = lastSegment.slice(dotIndex + 1).toLowerCase();
  return extensions.includes(extension);
}

function startsWithAny(pathname: string, prefixes: readonly string[]): boolean {
  return prefixes.some((prefix) => prefix.length > 0 && pathname.startsWith(prefix));
}

export function decideCacheStrategyDecision(
  request: CacheStrategyRequest,
  options: CacheStrategyOptions,
): CacheStrategyDecision {
  const {
    appOrigin,
    apiPathPrefixes = DEFAULT_API_PATH_PREFIXES,
    navigationFallback,
    neverCachedPathPrefixes = [],
    staticPathPrefixes = [],
    staticDestinations = DEFAULT_STATIC_DESTINATIONS,
    staticExtensions = DEFAULT_STATIC_EXTENSIONS,
    apiStrategy = CACHE_STRATEGIES.networkFirst,
    staticStrategy = CACHE_STRATEGIES.cacheFirst,
    defaultStrategy = CACHE_STRATEGIES.networkFirst,
  } = options;

  const parsed = new URL(request.url, appOrigin);
  const { pathname } = parsed;
  const sameOrigin = parsed.origin === appOrigin;

  if (matchesAnyPathPrefix(pathname, neverCachedPathPrefixes)) {
    return { strategy: CACHE_STRATEGIES.networkOnly };
  }

  const method = (request.method ?? 'GET').toUpperCase();
  if (method !== 'GET') {
    return { strategy: CACHE_STRATEGIES.networkOnly };
  }

  if (matchesAnyPathPrefix(pathname, apiPathPrefixes)) {
    return { strategy: apiStrategy };
  }

  if (request.mode === 'navigate') {
    if (navigationFallback !== undefined && sameOrigin) {
      return { strategy: defaultStrategy, navigationFallback };
    }
    return { strategy: defaultStrategy };
  }

  const staticAsset =
    (request.destination !== undefined && staticDestinations.includes(request.destination)) ||
    hasStaticExtension(pathname, staticExtensions) ||
    startsWithAny(pathname, staticPathPrefixes);

  return { strategy: sameOrigin && staticAsset ? staticStrategy : defaultStrategy };
}

export function decideCacheStrategy(
  request: CacheStrategyRequest,
  options: CacheStrategyOptions,
): CacheStrategy {
  return decideCacheStrategyDecision(request, options).strategy;
}

export function createCacheStrategyDecider(
  options: CacheStrategyOptions,
): (request: CacheStrategyRequest) => CacheStrategy {
  return (request) => decideCacheStrategy(request, options);
}

export type ApiOriginErrorReason = 'invalid_url' | 'insecure_scheme' | 'not_an_origin';

const REASON_MESSAGES = {
  invalid_url: 'must be an absolute URL',
  insecure_scheme: 'must use https, or http on a loopback host when that is allowed',
  not_an_origin: 'must be an origin without credentials, path, query, or fragment',
} as const satisfies Record<ApiOriginErrorReason, string>;

/** Names the setting and the rule it broke, never the configured value. */
export class ApiOriginError extends TypeError {
  public readonly reason: ApiOriginErrorReason;

  public constructor(label: string, reason: ApiOriginErrorReason) {
    super(`${label} ${REASON_MESSAGES[reason]}.`);
    this.name = 'ApiOriginError';
    this.reason = reason;
  }
}

export interface ApiOriginOptions {
  /** Permits plain HTTP only for localhost and literal loopback addresses. Defaults to false. */
  readonly allowLoopbackHttp?: boolean;
  /** Setting name used in error messages. Defaults to `API URL`. */
  readonly label?: string;
}

/**
 * Parses a configured API base URL and returns its origin, such as
 * `https://api.example.com`. Surrounding whitespace and one trailing slash are
 * accepted; anything beyond the origin is rejected.
 */
export function parseApiOrigin(value: string, options: ApiOriginOptions = {}): string {
  const label = options.label ?? 'API URL';
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    throw new ApiOriginError(label, 'invalid_url');
  }
  if (!isAllowedWebScheme(url, options.allowLoopbackHttp ?? false)) {
    throw new ApiOriginError(label, 'insecure_scheme');
  }
  if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw new ApiOriginError(label, 'not_an_origin');
  }
  return url.origin;
}

/** True for https, and for http on a loopback host when `allowLoopbackHttp` is set. */
export function isAllowedWebScheme(url: URL, allowLoopbackHttp: boolean): boolean {
  if (url.protocol === 'https:') return true;
  return url.protocol === 'http:' && allowLoopbackHttp && isLoopbackHost(url.hostname);
}

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/gu, '').toLowerCase();
  return host === 'localhost' || host === '::1' || /^127(?:\.\d{1,3}){3}$/u.test(host);
}

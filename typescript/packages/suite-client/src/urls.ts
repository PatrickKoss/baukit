import { createReturnUrlValidator } from '@baukit/integrations-client';

export interface SuiteReturnPolicy {
  readonly platform: 'web' | 'native';
  readonly origin: string;
}
export function suiteReturnUrl(policy: SuiteReturnPolicy): string {
  return policy.platform === 'web' ? `${policy.origin}/suite/linked` : `${policy.origin}/linked`;
}
export function suiteReturnValidator(policy: SuiteReturnPolicy): (url: string) => boolean {
  return createReturnUrlValidator({
    origin: policy.origin,
    paths: [policy.platform === 'web' ? '/suite/linked' : '/linked'],
  });
}
/** Allows only the two suite pages. Other native intents belong to the product. */
export function createSuiteNativeIntentValidator(scheme: string): (value: string) => boolean {
  if (!/^[a-z][a-z0-9+.-]*$/.test(scheme)) throw new TypeError('Invalid app scheme.');
  const routes = new Set([
    `${scheme}://suite/authorize`,
    `${scheme}://suite/linked`,
    `${scheme}:///suite/authorize`,
    `${scheme}:///suite/linked`,
    '/suite/authorize',
    '/suite/linked',
  ]);
  return (value) => routes.has(value.split(/[?#]/, 1)[0] ?? '');
}
export async function openSuitePeer(
  peer: { readonly scheme: string; readonly webUrl: string },
  options: {
    readonly platform: 'web' | 'native';
    readonly openUrl: (url: string) => Promise<void>;
    readonly allowLoopback?: boolean;
  },
): Promise<void> {
  const web = new URL(peer.webUrl);
  const loopback =
    options.allowLoopback === true && ['localhost', '127.0.0.1', '[::1]'].includes(web.hostname);
  if (
    !/^[a-z][a-z0-9+.-]*$/.test(peer.scheme) ||
    web.username !== '' ||
    web.password !== '' ||
    (web.protocol !== 'https:' && !(web.protocol === 'http:' && loopback))
  ) {
    throw new TypeError('Invalid peer URL.');
  }
  if (options.platform === 'native') {
    try {
      await options.openUrl(`${peer.scheme}://`);
      return;
    } catch {
      await options.openUrl(peer.webUrl);
      return;
    }
  }
  await options.openUrl(peer.webUrl);
}

export interface SuiteAuthorizePeer {
  readonly webUrl: string;
  /** Full hosted consent URL, including the authorizing product ID. */
  readonly authorizeUrl?: string;
}

/** Validate peer configuration before starting a connection. */
export function validateSuiteAuthorizePeer(
  peer: SuiteAuthorizePeer,
  options: { readonly allowLoopback?: boolean } = {},
): void {
  peerAuthorizeEndpoint(peer, options.allowLoopback);
}

function peerAuthorizeEndpoint(peer: SuiteAuthorizePeer, allowLoopback = false): URL {
  return authorizeEndpoint(
    peer.authorizeUrl ?? `${peer.webUrl.replace(/\/+$/, '')}/suite/authorize`,
    allowLoopback,
  );
}

function hasInvalidUrlCharacters(value: string): boolean {
  for (const character of value) {
    if (character <= ' ' || character === '\u007f' || character === '\\') return true;
  }
  return false;
}

function authorizeEndpoint(value: string, allowLoopback = false): URL {
  const url = new URL(value);
  const authority = value.split('://')[1]?.split(/[/?#]/, 1)[0];
  const loopback =
    url.hostname === 'localhost' ||
    url.hostname === '[::1]' ||
    /^127\.(?:\d{1,3}\.){2}\d{1,3}$/.test(url.hostname);
  if (
    authority === undefined ||
    authority.includes('@') ||
    hasInvalidUrlCharacters(value) ||
    url.username !== '' ||
    url.password !== '' ||
    value.includes('?') ||
    value.includes('#') ||
    (url.protocol !== 'https:' && !(url.protocol === 'http:' && allowLoopback && loopback))
  ) {
    throw new TypeError('Invalid suite authorize URL.');
  }
  return url;
}

/** Keep protocol query names identical for local and hosted consent pages. */
export function suiteAuthorizeUrl(
  peer: SuiteAuthorizePeer,
  input: {
    readonly client: string;
    readonly state: string;
    readonly codeChallenge: string;
    readonly hint?: string;
    readonly hintDomain?: string;
  },
  options: { readonly allowLoopback?: boolean } = {},
): string {
  const url = peerAuthorizeEndpoint(peer, options.allowLoopback);
  url.searchParams.append('client', input.client);
  url.searchParams.append('state', input.state);
  url.searchParams.append('code_challenge', input.codeChallenge);
  if (input.hint !== undefined) url.searchParams.append('hint', input.hint);
  if (input.hintDomain !== undefined) url.searchParams.append('hint_domain', input.hintDomain);
  return url.toString();
}

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

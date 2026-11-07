export interface PlansLinkContext {
  /** The product the visitor comes from, for example `eigenruhe`. */
  readonly from?: string;
  /** The locked feature key, for example `eigenruhe.studio`. Dropped without `from`. */
  readonly feature?: string;
}

// React Native's URL polyfill cannot edit search parameters, so this builds the string.
const httpUrl = /^https?:\/\/[^\s/?#@]+(?:[/?]\S*)?$/u;

/** The hub Plans link `<plansUrl>?from=<product>&feature=<key>`. Both are optional, and the hub ignores `feature` without `from`, so it is sent only with `from`. */
export function plansLink(plansUrl: string, context: PlansLinkContext = {}): string {
  if (!httpUrl.test(plansUrl)) throw new TypeError('The plans URL must be an HTTP(S) URL.');
  const parameters: string[] = [];
  const { from, feature } = context;
  if (from === undefined || from === '') return plansUrlWith(plansUrl, parameters);
  parameters.push(`from=${encodeURIComponent(from)}`);
  if (feature !== undefined && feature !== '')
    parameters.push(`feature=${encodeURIComponent(feature)}`);
  return plansUrlWith(plansUrl, parameters);
}

function plansUrlWith(plansUrl: string, parameters: readonly string[]): string {
  const query = parameters.join('&');
  if (query === '') return plansUrl;
  return `${plansUrl}${plansUrl.includes('?') ? '&' : '?'}${query}`;
}

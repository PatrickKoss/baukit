/**
 * Content Security Policy for a built web app (ADR 0006 section 7). Pure, so
 * build plugins, build checks and tests share one definition.
 */

/** `files`: CSS ships as files. `runtime-injected`: the UI library writes `<style>` at runtime. */
export type StyleSources = 'files' | 'runtime-injected';

export interface PolicyOrigins {
  readonly api?: string | undefined;
  readonly issuer?: string | undefined;
  readonly analytics?: string | undefined;
  readonly media?: string | undefined;
  readonly suiteHub?: string | undefined;
  readonly vendorConnect?: readonly string[];
  readonly vendorImages?: readonly string[];
  /** Origins added to `script-src`, for vendor scripts such as the Clerk frontend API. */
  readonly vendorScripts?: readonly string[];
  /** Origins added to `frame-src` next to the issuer. */
  readonly vendorFrames?: readonly string[];
}

export interface PolicyOptions {
  readonly styles: StyleSources;
  readonly origins: PolicyOrigins;
  /** `'sha256-...'` sources for every inline `<script>` the build emits. */
  readonly scriptHashes: readonly string[];
  /** Adds `blob:` to `frame-src`, for in-app previews that render a blob URL in an iframe. */
  readonly blobFrames?: boolean;
}

const SELF = "'self'";
const NONE = "'none'";
const INLINE = "'unsafe-inline'";
const INLINE_SCRIPT = /<script\b(?![^>]*\bsrc\s*=)[^>]*>([\s\S]*?)<\/script\s*>/giu;
const HASH_SOURCE = /^'sha256-[A-Za-z0-9+/]+={0,2}'$/u;

/** Returns the origin of a configured URL, or `undefined` when it is unset. */
export function originOf(value: string | undefined): string | undefined {
  if (value === undefined || value.trim() === '') {
    return undefined;
  }
  const url = new URL(value.includes('://') ? value : `https://${value}`);
  return url.origin;
}

/** The text of every inline script, in document order. */
export function inlineScripts(html: string): string[] {
  return [...html.matchAll(INLINE_SCRIPT)].map((match) => match[1] ?? '');
}

/** Formats a base64 SHA-256 digest as a CSP source. */
export function hashSource(base64Digest: string): string {
  const source = `'sha256-${base64Digest}'`;
  if (!HASH_SOURCE.test(source)) {
    throw new Error(`not a base64 SHA-256 digest: ${base64Digest}`);
  }
  return source;
}

function sources(...values: readonly (string | undefined)[]): string[] {
  return [...new Set(values.filter((value): value is string => value !== undefined))];
}

function originList(origins: PolicyOrigins): (string | undefined)[] {
  return [
    origins.api,
    origins.issuer,
    origins.analytics,
    origins.media,
    origins.suiteHub,
    ...(origins.vendorConnect ?? []),
    ...(origins.vendorImages ?? []),
    ...(origins.vendorScripts ?? []),
    ...(origins.vendorFrames ?? []),
  ];
}

function usesPlainHttp(origins: PolicyOrigins): boolean {
  return originList(origins).some((origin) => origin?.startsWith('http:') === true);
}

function frameSources(options: PolicyOptions): string[] {
  const { origins } = options;
  const list = sources(
    origins.issuer,
    ...(origins.vendorFrames ?? []),
    options.blobFrames === true ? 'blob:' : undefined,
  );
  return list.length === 0 ? [NONE] : list;
}

function directives(options: PolicyOptions): [string, string[]][] {
  const { origins } = options;
  const issuer = origins.issuer;
  return [
    ['default-src', [SELF]],
    ['script-src', sources(SELF, ...(origins.vendorScripts ?? []), ...options.scriptHashes)],
    ['style-src', options.styles === 'files' ? [SELF] : [SELF, INLINE]],
    ['img-src', sources(SELF, 'data:', 'blob:', origins.media, ...(origins.vendorImages ?? []))],
    ['media-src', sources(SELF, 'blob:', origins.media)],
    [
      'connect-src',
      sources(
        SELF,
        origins.api,
        issuer,
        origins.analytics,
        origins.suiteHub,
        ...(origins.vendorConnect ?? []),
      ),
    ],
    ['frame-src', frameSources(options)],
    ['worker-src', [SELF]],
    ['manifest-src', [SELF]],
    ['object-src', [NONE]],
    ['base-uri', [NONE]],
    ['form-action', sources(SELF, issuer)],
    ['frame-ancestors', [NONE]],
  ];
}

function serialize(entries: readonly [string, readonly string[]][], upgrade: boolean): string {
  const parts = entries.map(([name, values]) => `${name} ${values.join(' ')}`);
  return (upgrade ? [...parts, 'upgrade-insecure-requests'] : parts).join('; ');
}

/**
 * The production policy. `upgrade-insecure-requests` is left out only when an
 * origin is plain `http:`, which happens for local builds against localhost.
 */
export function contentSecurityPolicy(options: PolicyOptions): string {
  for (const hash of options.scriptHashes) {
    if (!HASH_SOURCE.test(hash)) {
      throw new Error(`not a CSP hash source: ${hash}`);
    }
  }
  return serialize(directives(options), !usesPlainHttp(options.origins));
}

/**
 * The dev server variant: inline styles and the HMR websocket. Never served
 * for a production build.
 */
export function developmentContentSecurityPolicy(
  options: PolicyOptions,
  hmrOrigins: readonly string[],
): string {
  const entries = directives({ ...options, styles: 'runtime-injected' }).map(
    ([name, values]): [string, string[]] =>
      name === 'connect-src' ? [name, sources(...values, ...hmrOrigins)] : [name, values],
  );
  return serialize(entries, false);
}

/** Inline scripts whose hash the policy's `script-src` lacks, as CSP hash sources. */
export function unhashedInlineScripts(
  html: string,
  policy: string,
  hash: (script: string) => string,
): string[] {
  const scriptSrc = policy
    .split(';')
    .map((directive) => directive.trim().split(/\s+/u))
    .find(([name]) => name === 'script-src');
  const allowed = new Set(scriptSrc?.slice(1) ?? []);
  return inlineScripts(html)
    .map((script) => hashSource(hash(script)))
    .filter((source) => !allowed.has(source));
}

/** Decodes Clerk's frontend API host from a publishable key (`pk_<env>_<base64 host$>`). */
export function clerkFrontendApiOrigin(publishableKey: string | undefined): string | undefined {
  const encoded = publishableKey?.split('_')[2];
  if (encoded === undefined || encoded === '') {
    return undefined;
  }
  const host = atob(encoded).replace(/\$$/u, '');
  return originOf(`https://${host}`);
}

const IPV4_MAPPED_LOOPBACK = /^\[::ffff:7f[0-9a-f]{2}:/u;

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/u, '');
  return (
    host === 'localhost' ||
    host.endsWith('.localhost') ||
    host === '[::1]' ||
    host === '[::]' ||
    host === '0.0.0.0' ||
    host.startsWith('127.') ||
    IPV4_MAPPED_LOOPBACK.test(host)
  );
}

/**
 * Throws when a configured origin is not a plain HTTPS origin on a public host.
 * A production export calls it so a leaked local `.env` cannot ship `http:` or
 * loopback origins in the policy.
 */
export function assertProductionOrigins(origins: PolicyOrigins): void {
  for (const origin of originList(origins)) {
    if (origin === undefined) {
      continue;
    }
    const url = new URL(origin);
    if (url.protocol !== 'https:' || url.username !== '' || url.password !== '') {
      throw new Error(`not an HTTPS origin without credentials: ${origin}`);
    }
    if (isLoopbackHost(url.hostname)) {
      throw new Error(`loopback origin in a production policy: ${origin}`);
    }
  }
}

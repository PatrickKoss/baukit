import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { resolve } from 'node:path';

import {
  contentSecurityPolicy,
  developmentContentSecurityPolicy,
  hashSource,
  inlineScripts,
  type PolicyOrigins,
  SECURITY_HEADERS,
  type StyleSources,
} from '@baukit/csp';
import type { Plugin, ResolvedConfig } from 'vite';

import type { BuildEnv } from './origins.ts';

export const POLICY_FILE = 'csp.txt';
export const CSP_HEADER = 'Content-Security-Policy';

export interface CspPluginOptions {
  readonly styles: StyleSources;
  readonly origins: (env: BuildEnv, mode: string) => PolicyOrigins;
}

export function sha256Base64(text: string): string {
  return createHash('sha256').update(text, 'utf8').digest('base64');
}

export function scriptHashSources(html: string): string[] {
  return inlineScripts(html).map((script) => hashSource(sha256Base64(script)));
}

function assetText(source: string | Uint8Array): string {
  return typeof source === 'string' ? source : new TextDecoder().decode(source);
}

type NextHandler = (error?: unknown) => void;

function isHtml(response: ServerResponse): boolean {
  const type = response.getHeader('content-type');
  return typeof type === 'string' && type.startsWith('text/html');
}

/** Sets the dev policy on HTML responses once Vite has injected its own inline scripts. */
function developmentMiddleware(policyFor: (html: string) => string) {
  return (_request: IncomingMessage, response: ServerResponse, next: NextHandler): void => {
    const end = response.end.bind(response) as (...args: unknown[]) => ServerResponse;
    response.end = ((...args: unknown[]) => {
      const [chunk] = args;
      if (!response.headersSent && isHtml(response) && (typeof chunk === 'string' || chunk instanceof Buffer)) {
        response.setHeader(CSP_HEADER, policyFor(chunk.toString()));
      }
      return end(...args);
    }) as ServerResponse['end'];
    next();
  };
}

/**
 * Builds `dist/csp.txt` with a hash for every inline script, serves it from
 * `vite preview`, and serves the relaxed variant from the dev server only.
 */
export function contentSecurityPolicyPlugin(options: CspPluginOptions): Plugin {
  let config: ResolvedConfig | undefined;
  const origins = (): PolicyOrigins => {
    if (config === undefined) {
      throw new Error('The CSP plugin ran before Vite resolved its config.');
    }
    return options.origins(config.env, config.mode);
  };
  return {
    name: 'baukit-content-security-policy',
    enforce: 'post',
    configResolved(resolved) {
      config = resolved;
    },
    configureServer(server) {
      const hmrPort = server.config.server.hmr === false ? undefined : server.config.server.port;
      const hmrOrigins =
        hmrPort === undefined
          ? []
          : [`ws://localhost:${String(hmrPort)}`, `ws://127.0.0.1:${String(hmrPort)}`];
      server.middlewares.use(
        developmentMiddleware((html) =>
          developmentContentSecurityPolicy(
            { styles: options.styles, origins: origins(), scriptHashes: scriptHashSources(html) },
            hmrOrigins,
          ),
        ),
      );
    },
    async configurePreviewServer(server) {
      const outDir = resolve(server.config.root, server.config.build.outDir);
      const policy = (await readFile(resolve(outDir, POLICY_FILE), 'utf8')).trim();
      server.middlewares.use((_request, response, next) => {
        response.setHeader(CSP_HEADER, policy);
        for (const [name, value] of Object.entries(SECURITY_HEADERS)) {
          response.setHeader(name, value);
        }
        next();
      });
    },
    generateBundle(_outputOptions, bundle) {
      const hashes = Object.values(bundle)
        .filter((output) => output.type === 'asset' && output.fileName.endsWith('.html'))
        .flatMap((output) => (output.type === 'asset' ? scriptHashSources(assetText(output.source)) : []));
      this.emitFile({
        type: 'asset',
        fileName: POLICY_FILE,
        source: `${contentSecurityPolicy({ styles: options.styles, origins: origins(), scriptHashes: hashes })}\n`,
      });
    },
  };
}

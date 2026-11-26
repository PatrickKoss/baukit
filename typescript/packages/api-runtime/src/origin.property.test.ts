import fc from 'fast-check';
import { describe, expect, it, vi } from 'vitest';

import { MockFetch, createApiFetch } from './index.js';

const baseUrl = 'https://api.example.test/v1';
const base = new URL(baseUrl);

const segment = fc.constantFrom('items', 'v1', 'v10', '..', '.', '%2F', '%2e%2e', 'a b', '');
const host = fc.constantFrom('api.example.test', 'evil.example', 'api.example.test:8443');
const path = fc.array(segment, { maxLength: 4 }).map((parts) => parts.join('/'));
const input = fc.oneof(
  path.map((value) => `/${value}`),
  path,
  fc.tuple(host, path).map(([name, value]) => `//${name}/${value}`),
  fc.tuple(host, path).map(([name, value]) => `/\\${name}/${value}`),
  fc.tuple(host, path).map(([name, value]) => `\\\\${name}/${value}`),
  fc
    .tuple(fc.constantFrom('https', 'http'), host, path)
    .map(([scheme, name, value]) => `${scheme}://${name}/${value}`),
);
const requestInput = fc.oneof(
  input,
  input
    .filter((value) => URL.canParse(value, `${baseUrl}/`))
    .map((value) => new Request(new URL(value, `${baseUrl}/`))),
);

function underBase(url: string): boolean {
  const target = new URL(url);
  return (
    target.origin === base.origin &&
    (target.pathname === base.pathname || target.pathname.startsWith(`${base.pathname}/`))
  );
}

function erasedResponse(): Response {
  return new Response(
    JSON.stringify({
      error: { code: 'profile_erased', message: 'x', requestId: 'r', details: {} },
    }),
    { status: 401, headers: { 'content-type': 'application/json' } },
  );
}

describe('C-01 api-runtime origin property', () => {
  it('attaches Authorization iff the request is under the base origin and path', async () => {
    await fc.assert(
      fc.asyncProperty(requestInput, async (value) => {
        const transport = new MockFetch().enqueue(new Response('{}'));
        const fetch = createApiFetch({
          baseUrl,
          environment: 'test',
          fetch: transport.fetch,
          tokenProvider: () => Promise.resolve('t'),
        });
        const outcome = await fetch(value).catch((cause: unknown) => cause);
        if (transport.requests.length === 0) {
          expect(outcome).toBeInstanceOf(Error);
          return;
        }
        const sent = transport.request(0);
        expect(sent.headers.get('authorization') !== null).toBe(underBase(sent.url));
      }),
    );
  });

  it('fires 401 hooks only for base requests and replays at most once', async () => {
    await fc.assert(
      fc.asyncProperty(requestInput, fc.boolean(), async (value, erased) => {
        const onProfileErased = vi.fn();
        const onUnauthorized = vi.fn().mockReturnValue('retry-once');
        const transport = new MockFetch();
        const sent: string[] = [];
        for (let index = 0; index < 3; index += 1) {
          transport.enqueue((request) => {
            sent.push(request.url);
            return erased ? erasedResponse() : new Response(null, { status: 401 });
          });
        }
        const fetch = createApiFetch({
          baseUrl,
          environment: 'test',
          fetch: transport.fetch,
          tokenProvider: () => Promise.resolve('t'),
          onProfileErased,
          onUnauthorized,
        });
        await expect(fetch(value)).rejects.toThrow();
        if (sent.length === 0) return;
        const trusted = underBase(sent[0] ?? '');
        expect(onProfileErased.mock.calls.length).toBe(trusted && erased ? 1 : 0);
        expect(onUnauthorized.mock.calls.length).toBe(trusted && !erased ? 1 : 0);
        expect(sent.length).toBeLessThanOrEqual(2);
        if (!trusted || erased) expect(sent.length).toBe(1);
      }),
    );
  });

  it('sends POST and PATCH without an idempotency key exactly once on 5xx', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.constantFrom('POST', 'PATCH'),
        fc.integer({ min: 500, max: 599 }),
        async (method, status) => {
          const transport = new MockFetch();
          for (let index = 0; index < 4; index += 1) {
            transport.enqueue(new Response(null, { status }));
          }
          const fetch = createApiFetch({
            baseUrl,
            environment: 'test',
            fetch: transport.fetch,
            retry: { methods: ['POST', 'PATCH'], sleep: () => Promise.resolve() },
          });
          await expect(fetch('/items', { method, body: '{}' })).rejects.toThrow();
          expect(transport.requests).toHaveLength(1);
        },
      ),
    );
  });
});

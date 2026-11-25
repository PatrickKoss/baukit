import { describe, expect, it, vi } from 'vitest';

import { ApiError, MockFetch, createApiFetch } from './index.js';

const baseUrl = 'https://api.example.test/v1';
const erased = () =>
  new Response(
    JSON.stringify({
      error: { code: 'profile_erased', message: 'gone', requestId: 'r', details: {} },
    }),
    { status: 401, headers: { 'content-type': 'application/json' } },
  );

function setup(sessionSubject?: () => string | undefined) {
  const transport = new MockFetch();
  const onProfileErased = vi.fn();
  const onUnauthorized = vi.fn().mockReturnValue('retry-once');
  const fetch = createApiFetch({
    baseUrl,
    environment: 'test',
    fetch: transport.fetch,
    tokenProvider: () => Promise.resolve('secret'),
    onProfileErased,
    onUnauthorized,
    ...(sessionSubject === undefined ? {} : { sessionSubject }),
  });
  return { transport, fetch, onProfileErased, onUnauthorized };
}

describe('base origin and path scoping', () => {
  it.each([
    ['/items', 'https://api.example.test/v1/items', true],
    ['items', 'https://api.example.test/v1/items', true],
    ['//evil.example/x', 'https://evil.example/x', false],
    ['/\\evil.example/x', 'https://evil.example/x', false],
    ['https://evil.example/v1/x', 'https://evil.example/v1/x', false],
    ['https://api.example.test/v2/x', 'https://api.example.test/v2/x', false],
    ['https://api.example.test/v10', 'https://api.example.test/v10', false],
    ['/../admin', 'https://api.example.test/admin', false],
  ])('resolves %s and attaches the token only under the base', async (input, url, trusted) => {
    const { transport, fetch } = setup();
    transport.enqueue(new Response('{}'));
    await fetch(input);
    const request = transport.request(0);
    expect(request.url).toBe(url);
    expect(request.headers.get('authorization')).toBe(trusted ? 'Bearer secret' : null);
  });

  it('applies the same rule to Request inputs', async () => {
    const { transport, fetch } = setup();
    transport.enqueue(new Response('{}')).enqueue(new Response('{}'));
    await fetch(new Request('https://evil.example/v1/x'));
    await fetch(new Request(`${baseUrl}/x`));
    expect(transport.request(0).headers.get('authorization')).toBeNull();
    expect(transport.request(1).headers.get('authorization')).toBe('Bearer secret');
  });

  it('ignores 401 signals from another origin', async () => {
    const { transport, fetch, onProfileErased, onUnauthorized } = setup();
    transport.enqueue(erased()).enqueue(new Response(null, { status: 401 }));
    await expect(fetch('https://evil.example/x')).rejects.toBeInstanceOf(ApiError);
    await expect(fetch('//evil.example/x')).rejects.toThrow();
    expect(onProfileErased).not.toHaveBeenCalled();
    expect(onUnauthorized).not.toHaveBeenCalled();
    transport.assertQueueEmpty();
  });
});

describe('session generation', () => {
  it('drops a profile_erased signal stamped with an older subject', async () => {
    let subject = 'account-a';
    const { transport, fetch, onProfileErased } = setup(() => subject);
    transport.enqueue(() => {
      subject = 'account-b';
      return erased();
    });
    await expect(fetch('/items')).rejects.toBeInstanceOf(ApiError);
    expect(onProfileErased).not.toHaveBeenCalled();
  });

  it('passes the stamped subject to onProfileErased', async () => {
    const { transport, fetch, onProfileErased } = setup(() => 'account-a');
    transport.enqueue(erased());
    await expect(fetch('/items')).rejects.toBeInstanceOf(ApiError);
    expect(onProfileErased).toHaveBeenCalledWith(expect.objectContaining({ subject: 'account-a' }));
  });
});

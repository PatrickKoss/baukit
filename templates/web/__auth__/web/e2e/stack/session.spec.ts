import { expect, test, type Route } from '@playwright/test';

import {
  apiUrl,
  expectNoToken,
  persistedBrowserState,
  recordIssuedTokens,
  refreshUrl,
  signedIn,
  signInButton,
  signInNewUser,
  waitsForRefreshLock,
} from './session';

const FOREIGN_ORIGIN = 'http://foreign.test';

test('a reload restores the session from the HttpOnly cookie and storage holds no token', async ({
  context,
  page,
}) => {
  const issuedTokens = recordIssuedTokens(context);
  const user = await signInNewUser(page);

  await page.reload();
  await expect(signedIn(page, user)).toBeVisible();

  const tokens = await issuedTokens();
  expect(tokens.length).toBeGreaterThanOrEqual(2);
  expectNoToken(await persistedBrowserState(page), tokens);
  const sessionCookies = (await context.cookies(apiUrl)).filter((cookie) => cookie.name.endsWith('-session'));
  expect(sessionCookies).toHaveLength(1);
  expect(sessionCookies[0]).toMatchObject({ httpOnly: true, sameSite: 'Strict' });
});

test('two tabs that reload together refresh once', async ({ context, page }) => {
  const user = await signInNewUser(page);
  const second = await context.newPage();
  await second.goto('/');
  await expect(signedIn(second, user)).toBeVisible();

  let refreshes = 0;
  let release: () => void = () => undefined;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await context.route(refreshUrl, async (route: Route) => {
    refreshes += 1;
    if (refreshes === 1) await held;
    await route.continue();
  });

  await page.reload();
  await expect.poll(() => refreshes).toBe(1);
  await second.reload();
  await expect.poll(() => waitsForRefreshLock(second)).toBe(true);
  release();

  await expect(signedIn(page, user)).toBeVisible();
  await expect(signedIn(second, user)).toBeVisible();
  expect(refreshes).toBe(1);
});

test('a reload while the API is unreachable keeps the account until it returns', async ({ context, page }) => {
  const user = await signInNewUser(page);
  await context.route(`${apiUrl}/**`, (route) => route.abort('internetdisconnected'));

  await page.reload();
  await expect(page.getByText(/^Signed in as /u)).toBeVisible();
  await expect(signInButton(page)).toHaveCount(0);

  await context.unroute(`${apiUrl}/**`);
  await page.reload();
  await expect(signedIn(page, user)).toBeVisible();
});

test('signing out in one tab signs out the other', async ({ context, page }) => {
  const user = await signInNewUser(page);
  const second = await context.newPage();
  await second.goto('/');
  await expect(signedIn(second, user)).toBeVisible();

  await page.getByRole('button', { name: 'Sign out' }).click();

  await expect(signInButton(second)).toBeVisible();
  await expect(signedIn(second, user)).toHaveCount(0);
  await second.reload();
  await expect(signInButton(second)).toBeVisible();
});

test('a foreign origin cannot obtain a token from the session cookie', async ({ context, page }) => {
  await signInNewUser(page);
  const foreign = await context.newPage();
  await foreign.route(`${FOREIGN_ORIGIN}/**`, (route) =>
    route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>foreign</title>' }),
  );
  await foreign.goto(`${FOREIGN_ORIGIN}/`);

  const fetched = await foreign.evaluate(async (url) => {
    try {
      const response = await fetch(url, {
        method: 'POST',
        credentials: 'include',
        headers: { 'X-Baukit-Session': '1' },
      });
      return await response.text();
    } catch (error) {
      return `blocked: ${String(error)}`;
    }
  }, refreshUrl);
  expect(fetched).toMatch(/^blocked: /u);

  const posted = foreign.waitForResponse(refreshUrl);
  await foreign.evaluate((url) => {
    const form = document.createElement('form');
    form.method = 'POST';
    form.action = url;
    document.body.append(form);
    form.submit();
  }, refreshUrl);
  const formResponse = await posted;
  expect(formResponse.status()).toBe(403);
  expect(await formResponse.text()).not.toContain('accessToken');

  const forged = await context.request.post(refreshUrl, {
    headers: { Origin: FOREIGN_ORIGIN, 'X-Baukit-Session': '1' },
  });
  expect(forged.status()).toBe(403);
  expect(await forged.text()).not.toContain('accessToken');
});

test('API responses are not sniffed and session responses are not cached', async ({ context, page }) => {
  await signInNewUser(page);
  const response = await context.request.post(refreshUrl, {
    headers: { Origin: new URL(page.url()).origin, 'X-Baukit-Session': '1' },
  });
  expect(response.status()).toBe(200);
  expect(response.headers()['cache-control']).toBe('no-store');
  expect(response.headers()['x-content-type-options']).toBe('nosniff');
  const anonymous = await context.request.get(`${apiUrl}/me`, { headers: { Authorization: '' } });
  expect(anonymous.status()).toBe(401);
  expect(anonymous.headers()['x-content-type-options']).toBe('nosniff');
});

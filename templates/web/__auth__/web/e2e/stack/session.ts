import { expect, type BrowserContext, type Page } from '@playwright/test';
import { createKeycloakTestUser, signInWithKeycloak, type KeycloakTestUser } from '@baukit/auth-node/keycloak-testing';

import { stack } from './keycloak';

export const apiUrl = process.env['E2E_API_URL'] ?? 'http://localhost:{{ context.api_host_port }}';
export const refreshUrl = `${apiUrl}/auth/session/refresh`;
const JWT = /eyJ[\w-]+\.[\w-]+\.[\w-]+/u;

export function signedIn(page: Page, user: KeycloakTestUser) {
  return page.getByText(`Signed in as ${user.subject}`);
}

export function signInButton(page: Page) {
  return page.getByRole('button', { name: /^Sign in with / });
}

/** Signs a new Keycloak user in through the UI and returns them. */
export async function signInNewUser(page: Page): Promise<KeycloakTestUser> {
  const user = await createKeycloakTestUser(stack);
  await page.goto('/');
  await signInButton(page).click();
  await signInWithKeycloak(page, user, stack);
  await expect(signedIn(page, user)).toBeVisible();
  return user;
}

/** Everything the page persisted in Web Storage and IndexedDB, as one string. */
export function persistedBrowserState(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const storage = (store: Storage) =>
      Array.from({ length: store.length }, (_, index) => {
        const key = store.key(index) ?? '';
        return `${key}=${store.getItem(key) ?? ''}`;
      });
    const readStore = (database: IDBDatabase, name: string) =>
      new Promise<unknown[]>((resolve, reject) => {
        const request = database.transaction(name, 'readonly').objectStore(name).getAll();
        request.onsuccess = () => {
          resolve(request.result);
        };
        request.onerror = () => {
          reject(request.error ?? new Error(`could not read ${name}`));
        };
      });
    const openDatabase = (name: string) =>
      new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onsuccess = () => {
          resolve(request.result);
        };
        request.onerror = () => {
          reject(request.error ?? new Error(`could not open ${name}`));
        };
      });
    const records: unknown[] = [];
    for (const info of await indexedDB.databases()) {
      if (info.name === undefined) continue;
      const database = await openDatabase(info.name);
      for (const store of Array.from(database.objectStoreNames)) {
        records.push(info.name, store, await readStore(database, store));
      }
      database.close();
    }
    return JSON.stringify([storage(localStorage), storage(sessionStorage), records]);
  });
}

export function expectNoToken(state: string, accessTokens: readonly string[]): void {
  expect(state).not.toMatch(JWT);
  for (const token of accessTokens) {
    expect(state).not.toContain(token);
  }
}

const ISSUING_ROUTES = ['exchange', 'refresh'].map((route) => `${apiUrl}/auth/session/${route}`);

function accessTokenOf(body: unknown): string | undefined {
  return typeof body === 'object' && body !== null && 'accessToken' in body && typeof body.accessToken === 'string'
    ? body.accessToken
    : undefined;
}

/** Records every access token the browser session routes hand out in this context. */
export function recordIssuedTokens(context: BrowserContext): () => Promise<string[]> {
  const bodies: Promise<PromiseSettledResult<unknown>>[] = [];
  context.on('response', (response) => {
    if (ISSUING_ROUTES.includes(response.url()) && response.ok()) {
      bodies.push(
        response.json().then(
          (value: unknown) => ({ status: 'fulfilled', value }) as const,
          (reason: unknown) => ({ status: 'rejected', reason }) as const,
        ),
      );
    }
  });
  return async () =>
    (await Promise.all(bodies)).map((body) => {
      if (body.status === 'rejected') {
        throw new Error('could not read a browser session response', { cause: body.reason });
      }
      const token = accessTokenOf(body.value);
      if (token === undefined) {
        throw new Error('a browser session response carried no access token');
      }
      return token;
    });
}

/** True once another tab holds the refresh lock and this one waits for it. */
export function waitsForRefreshLock(page: Page): Promise<boolean> {
  return page.evaluate(async () => {
    const state = await navigator.locks.query();
    return (state.pending ?? []).some((lock) => lock.name?.endsWith(':refresh') === true);
  });
}

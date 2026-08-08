import { createApiRuntime } from '@baukit/api-runtime';
import type { FetchImplementation } from '@baukit/api-runtime';

import { authClient } from './auth';

export interface Item {
  readonly id: string;
  readonly name: string;
}

export interface CurrentUser {
  readonly id: string;
  readonly subject: string;
}

const configuredBaseUrl: unknown = import.meta.env['VITE_API_URL'];
const api = createApiRuntime({
  baseUrl: typeof configuredBaseUrl === 'string' ? configuredBaseUrl : 'http://localhost:8080',
  environment: import.meta.env.MODE,
});

export async function listItems(fetch: FetchImplementation = api.fetch): Promise<readonly Item[]> {
  const response = await fetch('/items');
  return parseItems(await response.json());
}

export async function currentUser(fetch: FetchImplementation = api.fetch): Promise<CurrentUser> {
  const accessToken = await authClient.accessToken();
  if (accessToken === undefined) {
    throw new Error('Sign in before calling the protected API.');
  }
  const response = await fetch('/me', {
    headers: { authorization: `Bearer ${accessToken}` },
  });
  const value: unknown = await response.json();
  if (!isCurrentUser(value)) {
    throw new TypeError('The API returned an invalid current-user response.');
  }
  return value;
}

function parseItems(value: unknown): readonly Item[] {
  if (!Array.isArray(value) || !value.every(isItem)) {
    throw new TypeError('The API returned an invalid items response.');
  }
  return value;
}

function isItem(value: unknown): value is Item {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as Record<string, unknown>)['id'] === 'string' &&
    typeof (value as Record<string, unknown>)['name'] === 'string'
  );
}

function isCurrentUser(value: unknown): value is CurrentUser {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as Record<string, unknown>)['id'] === 'string' &&
    typeof (value as Record<string, unknown>)['subject'] === 'string'
  );
}

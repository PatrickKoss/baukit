import Constants from 'expo-constants';
import { OriginCooldown, type FetchImplementation } from '@baukit/api-runtime';

import { createAuthenticatedApiRuntime } from './authenticated-api';
import { apiSessionSubject } from './api-session';
import { authClient } from './auth';
import { apiAvailability, trackApiAvailability } from './api-availability';

export interface Item {
  readonly id: string;
  readonly name: string;
}

export interface CurrentUser {
  readonly id: string;
  readonly subject: string;
}

const configuredBaseUrl: unknown = Constants.expoConfig?.extra?.['apiBaseUrl'];
const baseUrl =
  typeof configuredBaseUrl === 'string'
    ? configuredBaseUrl
    : __DEV__
      ? 'http://localhost:{{ context.api_host_port }}'
      : '';
/** One cooldown for every runtime that calls this API, so a `Retry-After` from any of them blocks all. */
export const apiCooldown = new OriginCooldown({}, Date.now);

const api = createAuthenticatedApiRuntime({
  auth: authClient,
  sessionSubject: apiSessionSubject,
  baseUrl,
  environment: __DEV__ ? 'development' : 'production',
  cooldown: apiCooldown,
});
const apiFetch = trackApiAvailability(api.fetch, apiAvailability);

export async function listItems(fetch: FetchImplementation = apiFetch): Promise<readonly Item[]> {
  const response = await fetch('/items');
  return parseItems(await response.json());
}

export async function currentUser(fetch: FetchImplementation = apiFetch): Promise<CurrentUser> {
  const response = await fetch('/me');
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

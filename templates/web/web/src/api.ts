import { createApiRuntime } from '@baukit/api-runtime';
import type { FetchImplementation } from '@baukit/api-runtime';

export interface Item {
  readonly id: string;
  readonly name: string;
}

const configuredBaseUrl: unknown = import.meta.env['VITE_API_URL'];
const api = createApiRuntime({
  baseUrl: typeof configuredBaseUrl === 'string' ? configuredBaseUrl : 'http://localhost:{{ context.api_host_port }}',
  environment: import.meta.env.MODE,
});

export async function listItems(fetch: FetchImplementation = api.fetch): Promise<readonly Item[]> {
  const response = await fetch('/items');
  return parseItems(await response.json());
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

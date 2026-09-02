import { describe, expect, it } from 'vitest';

import { connectionStateFromServer } from './connection-health.js';
import { createProviderRegistry } from './provider-registry.js';

describe('provider registry', () => {
  it('keeps product labels and capabilities with derived connection actions', () => {
    const registry = createProviderRegistry([
      {
        id: 'calendar',
        labelKey: 'integrations.calendar',
        capabilities: ['read_events', 'write_events'],
        connection: connectionStateFromServer({ state: 'healthy' }),
      },
      {
        id: 'storage',
        labelKey: 'integrations.storage',
        capabilities: ['read_files'],
        connection: connectionStateFromServer({ state: 'revoked' }),
      },
    ] as const);

    expect(registry.list()).toEqual([
      {
        id: 'calendar',
        labelKey: 'integrations.calendar',
        capabilities: ['read_events', 'write_events'],
        currentState: 'connected',
        availableActions: ['disconnect'],
      },
      {
        id: 'storage',
        labelKey: 'integrations.storage',
        capabilities: ['read_files'],
        currentState: 'needs_reconnect',
        availableActions: ['reconnect'],
      },
    ]);
    expect(registry.get('storage')?.capabilities).toEqual(['read_files']);
  });

  it('copies caller-owned arrays', () => {
    const capabilities: string[] = ['read'];
    const registry = createProviderRegistry([
      {
        id: 'documents',
        labelKey: 'integrations.documents',
        capabilities,
        connection: connectionStateFromServer({ state: 'healthy' }),
      },
    ]);
    capabilities.push('write');
    expect(registry.get('documents')?.capabilities).toEqual(['read']);
  });

  it('rejects duplicate product IDs', () => {
    expect(() =>
      createProviderRegistry([
        {
          id: 'duplicate',
          labelKey: 'first',
          capabilities: [],
          connection: connectionStateFromServer({ state: 'healthy' }),
        },
        {
          id: 'duplicate',
          labelKey: 'second',
          capabilities: [],
          connection: connectionStateFromServer({ state: 'disconnected' }),
        },
      ]),
    ).toThrow('Provider IDs must be unique.');
  });
});

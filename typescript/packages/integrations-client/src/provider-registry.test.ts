import { describe, expect, expectTypeOf, it } from 'vitest';

import { connectionStateFromServer } from './connection-health.js';
import { createProviderRegistry } from './provider-registry.js';

describe('provider registry', () => {
  it('keeps product data and connectors in registration order', () => {
    const startCalendarOAuth = () => Promise.resolve('calendar');
    const startStorageOAuth = () => Promise.resolve('storage');
    const registry = createProviderRegistry([
      {
        id: 'calendar',
        labelKey: 'integrations.calendar',
        capabilities: ['read_events', 'write_events'],
        connector: { startOAuth: startCalendarOAuth, icon: 'calendar-icon' },
        connection: connectionStateFromServer({ state: 'healthy' }),
      },
      {
        id: 'storage',
        labelKey: 'integrations.storage',
        capabilities: ['read_files'],
        connector: { startOAuth: startStorageOAuth, icon: 'storage-icon' },
        connection: connectionStateFromServer({ state: 'revoked' }),
      },
    ] as const);

    expect(registry.list().map(({ id }) => id)).toEqual(['calendar', 'storage']);
    expect(registry.get('calendar')).toMatchObject({
      currentState: 'connected',
      availableActions: ['disconnect'],
    });
    expect(registry.get('calendar')?.connector?.startOAuth).toBe(startCalendarOAuth);
    expect(registry.get('storage')).toMatchObject({
      currentState: 'needs_reconnect',
      availableActions: ['reconnect'],
    });
    expect(registry.get('storage')?.capabilities).toEqual(['read_files']);
    expectTypeOf(registry.get('calendar')?.connector?.startOAuth).toEqualTypeOf<
      (() => Promise<string>) | undefined
    >();
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

  it('overlays connection states without changing the original registry', () => {
    const registry = createProviderRegistry([
      {
        id: 'calendar',
        labelKey: 'integrations.calendar',
        capabilities: ['read_events'],
        connector: { startOAuth: () => Promise.resolve() },
      },
      {
        id: 'storage',
        labelKey: 'integrations.storage',
        capabilities: ['read_files'],
        connector: { startOAuth: () => Promise.resolve() },
      },
    ] as const);

    const connected = registry.withConnectionStates(
      new Map([['calendar', connectionStateFromServer({ state: 'healthy' })]]),
    );

    expect(registry.get('calendar')).toMatchObject({
      currentState: 'disconnected',
      availableActions: [],
    });
    expect(connected.get('calendar')).toMatchObject({
      currentState: 'connected',
      availableActions: ['disconnect'],
    });
    expect(connected.get('storage')).toMatchObject({
      currentState: 'disconnected',
      availableActions: [],
    });
    expect(connected.get('calendar')?.connector).toBe(registry.get('calendar')?.connector);
    expect(connected.list().map(({ id }) => id)).toEqual(['calendar', 'storage']);
  });

  it('accepts record state overlays and ignores unknown state IDs', () => {
    const registry = createProviderRegistry([
      {
        id: 'calendar',
        labelKey: 'integrations.calendar',
        capabilities: ['read_events'],
      },
    ] as const);
    const states = {
      calendar: connectionStateFromServer({ state: 'revoked' }),
      unknown: connectionStateFromServer({ state: 'healthy' }),
    };

    const connected = registry.withConnectionStates(states);

    expect(connected.get('calendar')).toMatchObject({
      currentState: 'needs_reconnect',
      availableActions: ['reconnect'],
    });
    expect(connected.get('unknown')).toBeUndefined();
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

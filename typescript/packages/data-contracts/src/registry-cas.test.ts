import { describe, expect, it } from 'vitest';

import {
  removeScopedPersistenceRegistryEntry,
  resolveScopedStore,
  type ScopedPersistenceRegistryStore,
} from './identity.js';

const digest = (value: string): Promise<string> =>
  Promise.resolve(
    value.endsWith(':a') ? 'a'.repeat(64) : value.endsWith(':b') ? 'b'.repeat(64) : 'c'.repeat(64),
  );

/** Two adapter objects over one document, each yielding between read and write like a tab. */
class SharedDocument {
  public value: string | null = null;
  public adapter(): ScopedPersistenceRegistryStore {
    return {
      read: async () => {
        await Promise.resolve();
        return this.value;
      },
      write: async (next) => {
        await Promise.resolve();
        this.value = next;
      },
      compareAndSet: async (expected, next) => {
        await Promise.resolve();
        if (this.value !== expected) return false;
        this.value = next;
        return true;
      },
    };
  }
}

describe('registry updates across adapter instances', () => {
  it('keeps every concurrently resolved subject', async () => {
    const shared = new SharedDocument();
    const [first, second] = [shared.adapter(), shared.adapter()];
    await Promise.all([
      resolveScopedStore({ namespace: 'ns', subject: 'a', registry: first, digest }),
      resolveScopedStore({ namespace: 'ns', subject: 'b', registry: second, digest }),
    ]);
    expect(shared.value).toContain('"a"');
    expect(shared.value).toContain('"b"');
  });

  it('keeps a removal that races a resolution', async () => {
    const shared = new SharedDocument();
    await resolveScopedStore({ namespace: 'ns', subject: 'a', registry: shared.adapter(), digest });
    await Promise.all([
      removeScopedPersistenceRegistryEntry({
        namespace: 'ns',
        subject: 'a',
        registry: shared.adapter(),
      }),
      resolveScopedStore({ namespace: 'ns', subject: 'c', registry: shared.adapter(), digest }),
    ]);
    expect(shared.value).not.toContain('"a"');
    expect(shared.value).toContain('"c"');
  });

  it('runs updates inside the adapter lock', async () => {
    const shared = new SharedDocument();
    const events: string[] = [];
    const registry: ScopedPersistenceRegistryStore = {
      ...shared.adapter(),
      withLock: async (operation) => {
        events.push('lock');
        const result = await operation();
        events.push('unlock');
        return result;
      },
    };
    await resolveScopedStore({ namespace: 'ns', subject: 'a', registry, digest });
    expect(events).toEqual(['lock', 'unlock']);
  });
});

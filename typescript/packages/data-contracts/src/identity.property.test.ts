import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import {
  deriveScopedStoreName,
  isPersistenceIdentityMismatchError,
  removeScopedPersistenceRegistryEntry,
  resolveScopedStore,
  type ScopedPersistenceRegistryStore,
} from './identity.js';

const hex = (value: string): string =>
  [...value].reduce((hash, char) => (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0, 7)
    .toString(16)
    .padStart(64, '0');
const digest = (value: string): Promise<string> => Promise.resolve(hex(value));
const identityPart = fc.oneof(
  fc.string({ unit: 'grapheme', minLength: 1, maxLength: 6 }),
  fc.constantFrom('a:b', ':', 'a', 'a:', ':b', '\u{1F600}', 'x\u{10FFFF}'),
).filter((value) => value.trim().length > 0);

describe('C-03 registry properties', () => {
  it('canonicalIdentity is injective, including ":" and astral characters', async () => {
    await fc.assert(
      fc.asyncProperty(identityPart, identityPart, identityPart, identityPart, async (n1, s1, n2, s2) => {
        fc.pre(n1 !== n2 || s1 !== s2);
        const seen: string[] = [];
        const capture = (value: string): Promise<string> => {
          seen.push(value);
          return digest(value);
        };
        await deriveScopedStoreName(n1, s1, capture);
        await deriveScopedStoreName(n2, s2, capture);
        expect(seen[0]).not.toBe(seen[1]);
      }),
    );
  });

  it('parses arbitrary JSON into a resolution or the identity error, never writing on corrupt input', async () => {
    await fc.assert(
      fc.asyncProperty(fc.oneof(fc.json(), fc.string()), async (serialized) => {
        let writes = 0;
        const registry: ScopedPersistenceRegistryStore = {
          read: () => Promise.resolve(serialized),
          write: () => {
            writes += 1;
            return Promise.resolve();
          },
        };
        try {
          await resolveScopedStore({ namespace: 'ns', subject: 's', registry, digest });
        } catch (cause) {
          expect(isPersistenceIdentityMismatchError(cause)).toBe(true);
          expect(writes).toBe(0);
        }
      }),
    );
  });

  it('two adapters over one store keep every resolution and every removal', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.scheduler(),
        fc.array(fc.tuple(fc.boolean(), fc.constantFrom('a', 'b', 'c', 'd'), fc.nat(1)), {
          maxLength: 10,
        }),
        async (scheduler, operations) => {
          let document: string | null = null;
          const adapter = (): ScopedPersistenceRegistryStore => ({
            read: () => scheduler.schedule(Promise.resolve(document)),
            write: async (next) => {
              await scheduler.schedule(Promise.resolve());
              document = next;
            },
            compareAndSet: async (expected, next) => {
              await scheduler.schedule(Promise.resolve());
              if (document !== expected) return false;
              document = next;
              return true;
            },
          });
          const adapters = [adapter(), adapter()];
          const results = operations.map(([resolve, subject, index]) => {
            const registry = adapters[index] ?? adapters[0];
            if (registry === undefined) throw new Error('missing adapter');
            return (
              resolve
                ? resolveScopedStore({ namespace: 'ns', subject, registry, digest })
                : removeScopedPersistenceRegistryEntry({ namespace: 'ns', subject, registry })
            ).then(
              () => true,
              () => false,
            );
          });
          const settled = await scheduler.waitFor(Promise.all(results));
          if (settled.some((ok) => !ok)) return;
          for (const subject of new Set(operations.map(([, name]) => name))) {
            const kinds = new Set(
              operations.filter(([, name]) => name === subject).map(([resolve]) => resolve),
            );
            if (kinds.size !== 1) continue;
            const registered = (document ?? '').includes(`"subject":"${subject}"`);
            expect(registered).toBe(kinds.has(true));
          }
        },
      ),
      { numRuns: 200 },
    );
  });
});

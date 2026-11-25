import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { InMemoryScopedPersistenceRegistryStore } from './identity.js';
import { InMemoryStorePool } from './memory.js';
import { ScopedPersistenceLifecycle } from './scoped-lifecycle.js';

const subjects = ['account-a', 'account-b', 'account-c'] as const;
const digest = (value: string): Promise<string> =>
  Promise.resolve((value.at(-1) ?? 'x').charCodeAt(0).toString(16).padStart(64, '0'));

interface Real {
  readonly lifecycle: ScopedPersistenceLifecycle<{ close(): Promise<void> }>;
  readonly pending: Promise<unknown>[];
  openHandles: number;
  maxOpenHandles: number;
  readonly erased: string[];
  readonly allowedErasures: Set<string>;
  lastRequested: string | undefined;
  failures: boolean[];
}

type Model = object;

function setup(scheduler: fc.Scheduler, failures: boolean[]): Real {
  const pool = new InMemoryStorePool<{ id: string }>();
  const real: Real = {
    pending: [],
    openHandles: 0,
    maxOpenHandles: 0,
    erased: [],
    allowedErasures: new Set(),
    lastRequested: undefined,
    failures,
    lifecycle: undefined as never,
  };
  const step = async (label: string): Promise<void> => {
    await scheduler.schedule(Promise.resolve(label));
    if (real.failures.shift() === true) throw new Error(`${label} failed`);
  };
  Object.assign(real, {
    lifecycle: new ScopedPersistenceLifecycle({
      namespace: 'property',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: async ({ storeName }) => {
        await step('open');
        pool.open(storeName);
        real.openHandles += 1;
        real.maxOpenHandles = Math.max(real.maxOpenHandles, real.openHandles);
        return {
          close: async () => {
            real.openHandles -= 1;
            await step('close');
          },
        };
      },
      erase: async ({ subject }) => {
        await step('erase');
        real.erased.push(subject);
      },
      resetUserScopedState: () => undefined,
    }),
  });
  return real;
}

const track = (real: Real, operation: Promise<unknown>): void => {
  real.pending.push(operation.catch(() => undefined));
};

class Select implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly subject: string) {}
  public check = (): boolean => true;
  public run(_model: Model, real: Real): Promise<void> {
    real.lastRequested = this.subject;
    track(real, real.lifecycle.selectSubject(this.subject));
    return Promise.resolve();
  }
  public toString = (): string => `select(${this.subject})`;
}

class Clear implements fc.AsyncCommand<Model, Real> {
  public check = (): boolean => true;
  public run(_model: Model, real: Real): Promise<void> {
    real.lastRequested = undefined;
    track(real, real.lifecycle.clear());
    return Promise.resolve();
  }
  public toString = (): string => 'clear';
}

class EraseActive implements fc.AsyncCommand<Model, Real> {
  public check = (): boolean => true;
  public run(_model: Model, real: Real): Promise<void> {
    const subject = real.lifecycle.current()?.subject;
    if (subject !== undefined) {
      real.allowedErasures.add(subject);
      real.lastRequested = undefined;
    }
    track(real, real.lifecycle.eraseActivePartition());
    return Promise.resolve();
  }
  public toString = (): string => 'eraseActive';
}

class ProfileErased implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly subject: string) {}
  public check = (): boolean => true;
  public run(_model: Model, real: Real): Promise<void> {
    real.allowedErasures.add(this.subject);
    if (real.lastRequested === this.subject) real.lastRequested = undefined;
    track(real, real.lifecycle.handleProfileErased(this.subject));
    return Promise.resolve();
  }
  public toString = (): string => `profileErased(${this.subject})`;
}

class SessionExpired implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly subject: string) {}
  public check = (): boolean => true;
  public run(_model: Model, real: Real): Promise<void> {
    if (real.lastRequested === this.subject) real.lastRequested = undefined;
    track(real, real.lifecycle.handleSessionExpired(this.subject));
    return Promise.resolve();
  }
  public toString = (): string => `sessionExpired(${this.subject})`;
}

const subject = fc.constantFrom(...subjects);
type LifecycleCommand = fc.AsyncCommand<Model, Real>;
const command = (create: () => LifecycleCommand): fc.Arbitrary<LifecycleCommand> =>
  fc.constant(null).map(create);
const commands = fc.commands<Model, Real, false>(
  [
    subject.map((value): LifecycleCommand => new Select(value)),
    command(() => new Clear()),
    command(() => new EraseActive()),
    subject.map((value): LifecycleCommand => new ProfileErased(value)),
    subject.map((value): LifecycleCommand => new SessionExpired(value)),
  ],
  { maxCommands: 12 },
);

describe('C-02 ScopedPersistenceLifecycle under interleavings', () => {
  it('keeps one open handle, honours the last request and erases only signalled subjects', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.scheduler(),
        commands,
        fc.array(fc.boolean(), { maxLength: 30 }),
        async (scheduler, program, failures) => {
          const real = setup(scheduler, failures);
          await fc.scheduledModelRun(scheduler, () => ({ model: {}, real }), program);
          await scheduler.waitFor(Promise.all(real.pending));

          expect(real.maxOpenHandles).toBeLessThanOrEqual(1);
          for (const erased of real.erased) expect(real.allowedErasures).toContain(erased);
          const current = real.lifecycle.current();
          if (current !== undefined) expect(current.subject).toBe(real.lastRequested);
        },
      ),
      { numRuns: 300 },
    );
  });
});

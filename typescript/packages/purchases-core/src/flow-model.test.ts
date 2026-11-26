import fc from 'fast-check';
import { describe, it } from 'vitest';
import type { VerifiedEntitlement } from './entitlement.js';
import { PurchaseFlow } from './flow.js';
import { createNoopPurchasesPort } from './noop.js';

const SUBJECTS = ['suite:alice', 'suite:bob'] as const;
type Subject = (typeof SUBJECTS)[number];

interface Model {
  current: Subject | null;
  server: Map<Subject, VerifiedEntitlement | null>;
  /** The last answer the flow read for the current identity, `undefined` before any read. */
  read: VerifiedEntitlement | null | undefined;
}
interface Real {
  flow: PurchaseFlow;
  server: Map<Subject, VerifiedEntitlement | null>;
}

function plan(subject: Subject, revision: number, paid: boolean): VerifiedEntitlement {
  return {
    mode: 'hub',
    suiteSubject: subject,
    tier: paid ? 'premium' : 'free',
    features: paid ? ['export'] : [],
    revision,
    plansUrl: null,
    billing: null,
  };
}

/** A grant in the flow implies the latest answer read for the current subject granted it. */
function check(model: Model, real: Real): void {
  const held = real.flow.getState().entitlement;
  if (held === null) return;
  if (held.suiteSubject !== model.current) throw new Error('grant for another subject');
  if (model.read?.revision !== held.revision || model.read.tier !== held.tier)
    throw new Error(`stale grant ${JSON.stringify(held)} after ${JSON.stringify(model.read)}`);
}

class Identify implements fc.AsyncCommand<Model, Real> {
  constructor(readonly subject: Subject) {}
  check = (): boolean => true;
  async run(model: Model, real: Real): Promise<void> {
    await real.flow.identify(this.subject);
    model.current = this.subject;
    model.read = undefined;
    check(model, real);
  }
  toString = (): string => `identify(${this.subject})`;
}

class ServerAnswers implements fc.AsyncCommand<Model, Real> {
  constructor(
    readonly subject: Subject,
    readonly answer: 'paid' | 'free' | 'none',
    readonly step: number,
  ) {}
  check = (): boolean => true;
  async run(model: Model, real: Real): Promise<void> {
    const previous = model.server.get(this.subject)?.revision ?? 0;
    const value =
      this.answer === 'none'
        ? null
        : plan(this.subject, previous + this.step, this.answer === 'paid');
    model.server.set(this.subject, value);
    real.server.set(this.subject, value);
    await Promise.resolve();
    check(model, real);
  }
  toString = (): string => `server(${this.subject}, ${this.answer}, +${String(this.step)})`;
}

class Refresh implements fc.AsyncCommand<Model, Real> {
  check = (model: Readonly<Model>): boolean => model.current !== null;
  async run(model: Model, real: Real): Promise<void> {
    const subject = model.current;
    if (subject === null) return;
    const answer = model.server.get(subject) ?? null;
    await real.flow.refreshEntitlement();
    if (answer === null || model.read === undefined || model.read === null) model.read = answer;
    else if (answer.revision >= model.read.revision) model.read = answer;
    check(model, real);
  }
  toString = (): string => 'refresh';
}

describe('PurchaseFlow command model (C-09)', () => {
  it('never shows a grant the latest read for the current subject did not give', async () => {
    const commands = [
      fc.constantFrom(...SUBJECTS).map((subject) => new Identify(subject)),
      fc
        .tuple(
          fc.constantFrom(...SUBJECTS),
          fc.constantFrom('paid' as const, 'free' as const, 'none' as const),
          fc.integer({ min: 0, max: 2 }),
        )
        .map(([subject, answer, step]) => new ServerAnswers(subject, answer, step)),
      fc.constant(new Refresh()),
    ];
    await fc.assert(
      fc.asyncProperty(fc.commands(commands, { maxCommands: 30 }), async (cmds) => {
        const server = new Map<Subject, VerifiedEntitlement | null>();
        const flow = new PurchaseFlow({
          port: createNoopPurchasesPort(),
          backend: {
            refreshEntitlement: () => {
              const subject = flow.getState().suiteSubject as Subject | null;
              return Promise.resolve(subject === null ? null : (server.get(subject) ?? null));
            },
          },
        });
        const real: Real = { flow, server };
        await fc.asyncModelRun(
          () => ({ model: { current: null, server: new Map(), read: undefined }, real }),
          cmds,
        );
        flow.dispose();
      }),
      { numRuns: 200 },
    );
  });
});

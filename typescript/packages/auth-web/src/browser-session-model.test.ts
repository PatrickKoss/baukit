import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { readEndSessionUrl } from './browser-session-support.js';
import { BrowserSessionClient } from './browser-session-client.js';
import { jsonResponse, type Browser, type Tab } from './test-support/browser-session.js';
import {
  ACCESS_TOKEN,
  CONFIG,
  LIFETIME_MS,
  ROTATED_TOKEN,
  sessionCalls,
  setup,
  signIn,
  storageEntries,
  type Backend,
} from './test-support/session-backend.js';

interface Model {
  signedIn: boolean;
  rejectRefresh: boolean;
}

interface Real {
  readonly client: BrowserSessionClient;
  readonly browser: Browser;
  readonly server: Backend;
}

function refreshCount(real: Real): number {
  return sessionCalls(real.server, 'refresh').length;
}

class Advance implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly milliseconds: number) {}
  public check(): boolean {
    return true;
  }
  public run(_: Model, real: Real): Promise<void> {
    real.browser.setNow(real.browser.now() + this.milliseconds);
    return Promise.resolve();
  }
  public toString(): string {
    return `advance(${String(this.milliseconds)})`;
  }
}

class SetRejectRefresh implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly reject: boolean) {}
  public check(): boolean {
    return true;
  }
  public run(model: Model, real: Real): Promise<void> {
    model.rejectRefresh = this.reject;
    real.server.refresh = this.reject
      ? () => jsonResponse({ error: { code: 'session_expired' } }, 401)
      : () =>
          jsonResponse({
            accessToken: ROTATED_TOKEN,
            expiresAt: real.browser.now() + LIFETIME_MS,
            expiresIn: LIFETIME_MS / 1000,
            subject: 'user-1',
          });
    return Promise.resolve();
  }
  public toString(): string {
    return `rejectRefresh(${String(this.reject)})`;
  }
}

class GetToken implements fc.AsyncCommand<Model, Real> {
  public constructor(private readonly force: boolean) {}
  public check(): boolean {
    return true;
  }
  public async run(model: Model, real: Real): Promise<void> {
    const before = refreshCount(real);
    const token = await real.client.getAccessToken({ forceRefresh: this.force });
    const refreshes = refreshCount(real) - before;
    if (!model.signedIn) {
      expect(token).toBeUndefined();
      expect(refreshes).toBe(0);
      return;
    }
    expect(refreshes).toBeLessThanOrEqual(1);
    if (this.force) expect(refreshes).toBe(1);
    if (refreshes === 1 && model.rejectRefresh) {
      expect(token).toBeUndefined();
      model.signedIn = false;
      return;
    }
    expect([ACCESS_TOKEN, ROTATED_TOKEN]).toContain(token);
  }
  public toString(): string {
    return `getToken(${String(this.force)})`;
  }
}

class Clear implements fc.AsyncCommand<Model, Real> {
  public check(): boolean {
    return true;
  }
  public run(model: Model, real: Real): Promise<void> {
    real.client.clearSession();
    model.signedIn = false;
    return Promise.resolve();
  }
  public toString(): string {
    return 'clear';
  }
}

const commands = [
  fc.integer({ min: 0, max: 2 * LIFETIME_MS }).map((ms) => new Advance(ms)),
  fc.boolean().map((reject) => new SetRejectRefresh(reject)),
  fc.boolean().map((force) => new GetToken(force)),
  fc.constant(new Clear()),
];

describe('BrowserSessionClient model', () => {
  it('matches the sign-in model for any command sequence', async () => {
    await fc.assert(
      fc.asyncProperty(fc.commands(commands, { maxCommands: 30 }), async (sequence) => {
        const { browser, server } = setup();
        const tab: Tab = browser.tab();
        const client = await signIn(tab, new BrowserSessionClient(CONFIG, tab.environment));
        const real: Real = { client, browser, server };
        await fc.asyncModelRun(
          () => ({ model: { signedIn: true, rejectRefresh: false }, real }),
          sequence,
        );
        expect(JSON.stringify(storageEntries(browser.localStorage))).not.toMatch(/eyJ/u);
        client.dispose();
      }),
      { numRuns: 60 },
    );
  });
});

describe('callback and logout parsing', () => {
  it('completes a callback only when the returned state matches', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.oneof(
          fc.constant(CONFIG.redirectUri),
          fc.webUrl({ withQueryParameters: true, withFragments: true }),
        ),
        fc.option(fc.oneof(fc.constant('<issued>'), fc.string()), { nil: undefined }),
        async (url, state) => {
          const { browser, server } = setup();
          const tab = browser.tab();
          const client = new BrowserSessionClient(CONFIG, tab.environment);
          await client.login();
          const target = new URL(url);
          target.searchParams.set('code', 'one-time-code');
          const issued = new URL(tab.navigations.at(-1) ?? '').searchParams.get('state');
          const sent = state === '<issued>' ? (issued ?? '') : state;
          if (sent !== undefined) target.searchParams.set('state', sent);
          tab.setCurrentUrl(target.toString());
          const outcome = await client.handleCallback().catch(() => 'rejected' as const);
          if (sessionCalls(server, 'exchange').length > 0) {
            expect(outcome).toBe(true);
            expect(sent).toBe(issued);
          } else {
            expect(outcome).not.toBe(true);
          }
          if (url === CONFIG.redirectUri && sent === issued) expect(outcome).toBe(true);
        },
      ),
      { numRuns: 100 },
    );
  });

  it('returns only http(s) end-session URLs', () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.string(),
          fc.webUrl(),
          fc.constantFrom('javascript:alert(1)', 'data:text/html,x', 'file:///etc/passwd'),
        ),
        (raw) => {
          const url = readEndSessionUrl({ endSessionUrl: raw });
          if (url !== undefined) expect(new URL(url).protocol).toMatch(/^https?:$/u);
        },
      ),
    );
    fc.assert(
      fc.property(fc.anything(), (value) => {
        const url = readEndSessionUrl(value);
        if (url !== undefined) expect(new URL(url).protocol).toMatch(/^https?:$/u);
      }),
    );
  });
});

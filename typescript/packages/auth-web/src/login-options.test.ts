import { describe, expect, it } from 'vitest';

import { BrowserSessionClient } from './index.js';
import { appearanceStateDecoration, decoratedAuthorizationState } from './login-options.js';
import { required } from './test-support/browser-session.js';
import { CONFIG, setup } from './test-support/session-backend.js';

const ENTROPY = new Uint8Array(32).fill(171);
const NONCE = 'ab'.repeat(32);

async function authorizationUrl(
  options: Parameters<BrowserSessionClient['login']>[0],
): Promise<URL> {
  const { browser } = setup();
  const tab = browser.tab();
  await new BrowserSessionClient(CONFIG, tab.environment).login(options);
  return new URL(required(tab.navigations.at(-1)));
}

describe('appearanceStateDecoration', () => {
  it('encodes mode and optional colors', () => {
    expect(appearanceStateDecoration({ mode: 'dark' })).toEqual(['ap1', 'd']);
    expect(
      appearanceStateDecoration({
        mode: 'system',
        primaryColor: '#0a1B2c',
        secondaryColor: '#FFFFFF',
      }),
    ).toEqual(['ap1', 's', '0A1B2C', 'FFFFFF']);
  });

  it.each([
    { mode: 'light', primaryColor: '#000000' },
    { mode: 'light', primaryColor: 'red', secondaryColor: '#000000' },
    { mode: 'light', primaryColor: '#00000', secondaryColor: '#000000' },
  ] as const)('rejects %j', (appearance) => {
    expect(() => appearanceStateDecoration(appearance)).toThrow(TypeError);
  });
});

describe('decoratedAuthorizationState', () => {
  it('joins the decoration and a hex nonce', () => {
    expect(decoratedAuthorizationState(['ap1', 'l'], ENTROPY)).toBe(`ap1.l.${NONCE}`);
  });

  it.each([[[]], [['a.b']], [['a b']], [['']], [['a/../b']], [['a&prompt=none']]])(
    'rejects decoration %j',
    (decoration) => {
      expect(() => decoratedAuthorizationState(decoration, ENTROPY)).toThrow(TypeError);
    },
  );

  it('rejects short entropy', () => {
    expect(() => decoratedAuthorizationState(['ap1'], new Uint8Array(31))).toThrow(RangeError);
  });
});

describe('BrowserSessionClient.login options', () => {
  it('keeps a plain login free of decoration and prompt', async () => {
    const url = await authorizationUrl(undefined);
    expect(url.searchParams.get('prompt')).toBeNull();
    expect(required(url.searchParams.get('state'))).not.toContain('.');
  });

  it('puts the decoration in front of the random nonce', async () => {
    const url = await authorizationUrl({
      stateDecoration: appearanceStateDecoration({ mode: 'dark' }),
    });
    expect(required(url.searchParams.get('state'))).toMatch(/^ap1\.d\.[0-9a-f]{64}$/u);
  });

  it('requests registration with prompt=create', async () => {
    const url = await authorizationUrl({ entry: 'register' });
    expect(url.searchParams.get('prompt')).toBe('create');
  });

  it('rejects a decoration that could smuggle query parameters', async () => {
    await expect(authorizationUrl({ stateDecoration: ['ap1&prompt=none'] })).rejects.toThrow(
      TypeError,
    );
  });
});

import type { Page } from '@playwright/test';
import { describe, expect, it } from 'vitest';

import {
  keycloakStack,
  signInWithKeycloak,
  type KeycloakLoginLocator,
  type KeycloakLoginPage,
} from './keycloak-testing.js';

const STACK = keycloakStack(
  { url: 'http://localhost:8081/', realm: 'notes', webClientId: 'notes-web' },
  {},
);
const BAUKIT_FORM = '#kc-form-login[data-baukit-accessibility-initialized]';
const USER = { username: 'ada', password: 'pw' };

interface FakeLoginOptions {
  readonly reauthentication?: boolean;
  readonly attemptedUsername?: string;
  readonly dropFilled?: string;
}

function fakePage(options: FakeLoginOptions = {}): {
  readonly page: KeycloakLoginPage;
  readonly steps: string[];
} {
  const steps: string[] = [];
  const values = new Map<string, string>([
    ['#kc-attempted-username', options.attemptedUsername ?? USER.username],
  ]);
  const onKeycloak = new URL('http://localhost:8081/realms/notes/login');
  const onApp = new URL('http://localhost:5173/');
  const field = (selector: string): KeycloakLoginLocator => ({
    waitFor: (wait) => {
      expect(wait.state).toBe('visible');
      steps.push(
        `ready ${selector} ${wait.timeout === undefined ? 'default' : String(wait.timeout)}`,
      );
      return Promise.resolve();
    },
    count: () => {
      const count = options.reauthentication === true && selector.endsWith('#username') ? 0 : 1;
      steps.push(`count ${selector} ${String(count)}`);
      return Promise.resolve(count);
    },
    fill: (value) => {
      steps.push(`fill ${selector} ${value}`);
      if (options.dropFilled !== selector) values.set(selector, value);
      return Promise.resolve();
    },
    inputValue: () => Promise.resolve(values.get(selector) ?? ''),
    click: () => {
      steps.push(`click ${selector}`);
      return Promise.resolve();
    },
  });
  const page: KeycloakLoginPage = {
    waitForURL: (matches, wait) => {
      const timeout = wait?.timeout === undefined ? 'default' : String(wait.timeout);
      steps.push(`wait ${String(matches(onKeycloak))} ${String(matches(onApp))} ${timeout}`);
      return Promise.resolve();
    },
    locator: field,
  };
  return { page, steps };
}

describe('signInWithKeycloak', () => {
  it('waits for the initialized baukit form and its autofocus before filling', async () => {
    const { page, steps } = fakePage();

    await signInWithKeycloak(page, USER, STACK, { timeoutMs: 60_000 });

    expect(steps).toEqual([
      'wait true false 60000',
      `ready ${BAUKIT_FORM} 60000`,
      `ready ${BAUKIT_FORM} input[autofocus]:focus 60000`,
      `count ${BAUKIT_FORM} #username 1`,
      `fill ${BAUKIT_FORM} #username ada`,
      `fill ${BAUKIT_FORM} #password pw`,
      `click ${BAUKIT_FORM} #kc-login`,
    ]);
  });

  it('waits for the plain form on a stock Keycloak theme', async () => {
    const { page, steps } = fakePage();

    await signInWithKeycloak(page, USER, STACK, { theme: 'keycloak' });

    expect(steps).toEqual([
      'wait true false default',
      'ready #kc-form-login default',
      'ready #kc-form-login input[autofocus]:focus default',
      'count #kc-form-login #username 1',
      'fill #kc-form-login #username ada',
      'fill #kc-form-login #password pw',
      'click #kc-form-login #kc-login',
    ]);
  });

  it('checks the attempted username on a password-only reauthentication form', async () => {
    const { page, steps } = fakePage({ reauthentication: true });

    await signInWithKeycloak(page, USER, STACK);

    expect(steps).toEqual([
      'wait true false default',
      `ready ${BAUKIT_FORM} default`,
      `ready ${BAUKIT_FORM} input[autofocus]:focus default`,
      `count ${BAUKIT_FORM} #username 0`,
      `fill ${BAUKIT_FORM} #password pw`,
      `click ${BAUKIT_FORM} #kc-login`,
    ]);
  });

  it('rejects a reauthentication form for another user', async () => {
    const { page, steps } = fakePage({ reauthentication: true, attemptedUsername: 'grace' });

    await expect(signInWithKeycloak(page, USER, STACK)).rejects.toThrow(
      'The Keycloak username field does not hold the expected value.',
    );
    expect(steps.some((step) => step.startsWith('click'))).toBe(false);
  });

  it('does not submit when the page drops a typed password', async () => {
    const { page, steps } = fakePage({ dropFilled: `${BAUKIT_FORM} #password` });

    await expect(signInWithKeycloak(page, USER, STACK)).rejects.toThrow(
      'The Keycloak password field does not hold the expected value.',
    );
    expect(steps.some((step) => step.startsWith('click'))).toBe(false);
  });

  it('accepts a Playwright page', () => {
    const asLoginPage = (page: Page): KeycloakLoginPage => page;
    expect(asLoginPage).toBeTypeOf('function');
  });
});

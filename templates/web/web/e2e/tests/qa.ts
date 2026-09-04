import { expect, type Locator, type Page, type TestInfo } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

import { qaConfig, type QaApiStub, type QaSubmitField, type QaSubmitTarget } from '../qa.config';

/** Serves every configured API path from a fixture, so no backend is required. */
export async function stubApi(page: Page, stubs: readonly QaApiStub[]): Promise<void> {
  for (const stub of stubs) {
    await page.route(stub.url, async (route) => {
      await route.fulfill({
        status: stub.status,
        contentType: 'application/json',
        body: JSON.stringify(stub.body),
      });
    });
  }
}

/** Fails every configured API path with 401, standing in for an expired session. */
export async function expireSession(page: Page, url: string): Promise<void> {
  await page.unroute(url);
  await page.route(url, async (route) => {
    await route.fulfill({
      status: 401,
      contentType: 'application/json',
      body: JSON.stringify({ title: 'Unauthorized' }),
    });
  });
}

export async function openRoute(page: Page, path: string, authenticated = false): Promise<void> {
  if (authenticated) {
    const authentication = qaConfig.authentication;
    if (authentication === undefined) {
      throw new Error('This QA case requires authentication, but no authentication state exists.');
    }
    await page.addInitScript((entries) => {
      for (const entry of entries) {
        localStorage.setItem(entry.key, entry.value);
      }
    }, authentication.localStorage);
  }
  await page.goto(path);
}

export function submitFields(target: QaSubmitTarget): readonly QaSubmitField[] {
  return 'fields' in target ? target.fields : [{ field: target.field, value: target.value }];
}

export function invalidSubmitField(target: QaSubmitTarget): string {
  return 'invalidField' in target ? target.invalidField : target.field;
}

/**
 * Blocking violations are the serious and critical ones. Everything axe reports
 * is attached so a failure names the offending nodes instead of a bare count.
 */
export async function expectNoBlockingAxeViolations(
  page: Page,
  testInfo: TestInfo,
  screen: string,
): Promise<void> {
  const results = await new AxeBuilder({ page }).analyze();
  const blocking = results.violations.filter(
    ({ impact }) => impact === 'serious' || impact === 'critical',
  );
  await testInfo.attach(`axe-${screen}`, {
    body: Buffer.from(
      JSON.stringify({ incomplete: results.incomplete, violations: blocking }, null, 2),
    ),
    contentType: 'application/json',
  });
  expect
    .soft(
      blocking.map(({ help, id, nodes }) => ({
        help,
        id,
        nodes: nodes.map(({ failureSummary, html, target }) => ({
          failureSummary,
          html,
          target,
        })),
      })),
      `${screen} has serious or critical axe violations`,
    )
    .toEqual([]);
}

/** Moves focus to the document body without activating anything. */
export async function resetFocusToBody(page: Page): Promise<void> {
  await page.evaluate(() => {
    document.body.setAttribute('tabindex', '-1');
    document.body.focus();
    document.body.removeAttribute('tabindex');
  });
}

/** Tabs `presses` times and fails if focus ever leaves the container subtree. */
export async function expectFocusStaysInside(
  page: Page,
  container: Locator,
  presses: number,
): Promise<void> {
  for (let press = 0; press < presses; press += 1) {
    await page.keyboard.press('Tab');
    await expect
      .poll(() => container.evaluate((element) => element.contains(document.activeElement)), {
        message: `Tab press ${String(press + 1)} escaped the container`,
      })
      .toBe(true);
  }
}

export interface FocusVisual {
  readonly boxShadow: string;
  readonly outlineColor: string;
  readonly outlineOffset: string;
  readonly outlineStyle: string;
  readonly outlineWidth: string;
}

export async function computedFocusVisual(locator: Locator): Promise<FocusVisual> {
  return locator.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      boxShadow: style.boxShadow,
      outlineColor: style.outlineColor,
      outlineOffset: style.outlineOffset,
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
    };
  });
}

/**
 * Two synchronous DOM clicks in one task. Deliberately not `dblclick`, so a
 * handler cannot debounce on event timing and still pass.
 */
export async function rapidDoubleClick(locator: Locator): Promise<void> {
  await locator.evaluate((element: HTMLElement) => {
    element.click();
    element.click();
  });
}

export interface Box {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

/** Box of the nearest ancestor that actually scrolls vertically. */
export async function scrollingAncestorBox(locator: Locator): Promise<Box> {
  return locator.evaluate((element) => {
    let candidate: HTMLElement | null = element as HTMLElement;
    while (candidate) {
      const { overflowY } = window.getComputedStyle(candidate);
      if (overflowY === 'auto' || overflowY === 'scroll') {
        const { height, width, x, y } = candidate.getBoundingClientRect();
        return { height, width, x, y };
      }
      candidate = candidate.parentElement;
    }
    const { height, width, x, y } = document.documentElement.getBoundingClientRect();
    return { height, width, x, y };
  });
}

import { afterEach, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { createRoot, type Root } from 'react-dom/client';
import { act, useState } from 'react';
import axe from 'axe-core';
import { exampleTokens, toCssVariables } from '@baukit/ui-tokens';
import { useReducedMotionPreference } from '@baukit/a11y-core/web';
import { AppNavigation, SectionPicker, type NavigationIcon } from './web.js';
import stylesheet from './web.css?raw';
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const icon: NavigationIcon = () => (
  <svg width="24" height="24" viewBox="0 0 24 24">
    <path d="M4 4h16v16H4z" fill="currentColor" />
  </svg>
);
const section = {
  id: 'progress',
  label: 'Progress',
  href: '/progress',
  icon,
  children: [
    { id: 'overview', label: 'Overview', href: '/progress' },
    { id: 'history', label: 'History', href: '/progress/history' },
  ],
};
const items = [{ id: 'home', label: 'Home', href: '/', icon }, section];
let root: Root | undefined;
let host: HTMLDivElement | undefined;
let styles: HTMLStyleElement | undefined;
function Fixture() {
  const [collapsed, setCollapsed] = useState(false);
  const { reducedMotion, resolved } = useReducedMotionPreference();
  return (
    <div>
      <AppNavigation
        items={items}
        pathname="/progress/history"
        collapsed={collapsed}
        onCollapsedChange={setCollapsed}
        profile={{ label: 'Account', initials: 'AB', href: '/profile' }}
      />
      <main
        style={{
          padding: '8px',
          marginLeft: window.innerWidth >= 1024 ? (collapsed ? 76 : 280) : 0,
          transition:
            resolved && !reducedMotion
              ? 'margin-left var(--bk-motion-duration-normal) var(--bk-motion-easing-standard)'
              : undefined,
          paddingBottom: 80,
        }}
      >
        <h1>Navigation fixture</h1>
        <SectionPicker item={section} pathname="/progress/history" />
        <button type="button" style={{ minWidth: 44, minHeight: 44 }}>
          Primary action
        </button>
      </main>
    </div>
  );
}
afterEach(async () => {
  await act(() => {
    root?.unmount();
    return Promise.resolve();
  });
  host?.remove();
  styles?.remove();
  delete document.documentElement.dataset['theme'];
  vi.restoreAllMocks();
});
it.each(
  [320, 1023, 1024].flatMap((width) =>
    [568, 720].flatMap((height) => ['light', 'dark'].map((theme) => ({ width, height, theme }))),
  ),
)(
  'lays out accessible $theme navigation at $width by $height',
  async ({ width, height, theme }) => {
    const errors: unknown[][] = [];
    vi.spyOn(console, 'warn').mockImplementation((...args: unknown[]) => {
      errors.push(args);
    });
    vi.spyOn(console, 'error').mockImplementation((...args: unknown[]) => {
      errors.push(args);
    });
    await page.viewport(width, height);
    document.documentElement.lang = 'en';
    document.title = 'Navigation fixture';
    document.documentElement.dataset['theme'] = theme;
    styles = document.createElement('style');
    styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} body {margin:0;background:var(--bk-color-background-primary);color:var(--bk-color-text-primary);} *{box-sizing:border-box;}`;
    document.head.append(styles);
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    await act(() => {
      root?.render(<Fixture />);
      return Promise.resolve();
    });
    const navigation = document.querySelector<HTMLElement>('.bk-navigation');
    if (navigation === null) throw new Error('Navigation did not render');
    expect(navigation.dataset['layout']).toBe(width >= 1024 ? 'rail' : 'bar');
    expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(width);
    const activeLink = page
      .getByRole('link', { name: width >= 1024 ? 'History' : 'Progress' })
      .element();
    function filledRows() {
      return Array.from(
        navigation?.querySelectorAll<HTMLElement>('.bk-navigation-item') ?? [],
      ).filter(
        (item) =>
          item.closest('[hidden]') === null &&
          getComputedStyle(item).backgroundColor !== 'rgba(0, 0, 0, 0)',
      );
    }
    expect(filledRows()).toEqual([activeLink]);
    expect(activeLink.getAttribute('data-active')).toBe('page');
    expect(getComputedStyle(activeLink).borderWidth).toBe('0px');
    expect(getComputedStyle(activeLink).fontWeight).toBe('700');
    await act(async () => {
      await userEvent.tab();
      activeLink.focus();
    });
    expect(activeLink.matches(':focus-visible')).toBe(true);
    const focusedStyle = getComputedStyle(activeLink);
    expect(focusedStyle.outlineColor).not.toBe(focusedStyle.backgroundColor);
    expect(Number.parseFloat(focusedStyle.outlineWidth)).toBeGreaterThanOrEqual(3);
    const profile = page.getByRole('link', { name: 'Account' }).element().getBoundingClientRect();
    const navBox = navigation.getBoundingClientRect();
    if (width >= 1024) {
      expect(navBox.width).toBe(280);
      expect(profile.bottom).toBeGreaterThan(height - 60);
      const parent = page.getByRole('button', { name: 'Progress' }).element();
      expect(parent.getAttribute('data-active')).toBe('ancestor');
      expect(getComputedStyle(parent).backgroundColor).toBe('rgba(0, 0, 0, 0)');
      expect(getComputedStyle(parent).color).toBe(getComputedStyle(activeLink).color);
      const group = parent.nextElementSibling;
      expect(group?.getAttribute('role')).toBe('group');
      const history = page.getByRole('link', { name: 'History' }).element().getBoundingClientRect();
      expect(history.top).toBeGreaterThanOrEqual(parent.getBoundingClientRect().bottom);
      expect(history.left).toBeGreaterThan(parent.getBoundingClientRect().left);
      await act(async () => {
        await page.getByRole('button', { name: 'Collapse navigation' }).click();
      });
      expect(
        page.getByRole('button', { name: 'Primary action' }).element().getBoundingClientRect().left,
      ).toBeGreaterThanOrEqual(navigation.getBoundingClientRect().right);
      await expect.poll(() => navigation.getBoundingClientRect().width).toBe(76);
      expect(filledRows()).toEqual([parent]);
      expect(parent.getAttribute('data-active')).toBe('page');
      expect(page.getByRole('button', { name: 'Progress' }).element().getAttribute('title')).toBe(
        'Progress',
      );
    } else {
      const progress = page
        .getByRole('link', { name: 'Progress' })
        .element()
        .getBoundingClientRect();
      expect(profile.left).toBeGreaterThanOrEqual(progress.right);
      expect(navBox.bottom).toBe(height);
    }
    const action = page
      .getByRole('button', { name: 'Primary action' })
      .element()
      .getBoundingClientRect();
    expect(
      width >= 1024
        ? action.left >= navigation.getBoundingClientRect().right
        : action.bottom <= navBox.top,
    ).toBe(true);
    for (const target of host.querySelectorAll<HTMLElement>('a, button')) {
      if (target.closest('[hidden]') !== null) continue;
      const box = target.getBoundingClientRect();
      expect(box.width).toBeGreaterThanOrEqual(44);
      expect(box.height).toBeGreaterThanOrEqual(44);
      expect(box.left).toBeGreaterThanOrEqual(0);
      expect(box.right).toBeLessThanOrEqual(width);
    }
    expect((await axe.run(host)).violations).toEqual([]);
    expect(errors).toEqual([]);
  },
);

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

const labels = {
  label: 'Primary',
  collapseLabel: 'Collapse navigation',
  expandLabel: 'Expand navigation',
  closeLabel: 'Close',
};

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
        {...labels}
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
        <SectionPicker closeLabel="Close" item={section} pathname="/progress/history" />
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
    expect(Number.parseFloat(focusedStyle.outlineOffset)).toBe(2);
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
      await act(async () => {
        await userEvent.tab();
        parent.focus();
      });
      expect(parent.matches(':focus-visible')).toBe(true);
      const parentStyle = getComputedStyle(parent);
      expect(Number.parseFloat(parentStyle.outlineOffset)).toBe(2);
      const ringExtent = Number.parseFloat(parentStyle.outlineWidth) + 2;
      const parentBox = parent.getBoundingClientRect();
      const scrollBox = navigation.querySelector('.bk-navigation-items')?.getBoundingClientRect();
      if (scrollBox === undefined) throw new Error('Navigation scroll container did not render');
      expect(parentBox.left - ringExtent).toBeGreaterThanOrEqual(scrollBox.left);
      expect(parentBox.right + ringExtent).toBeLessThanOrEqual(scrollBox.right);
      expect(parentBox.top - ringExtent).toBeGreaterThanOrEqual(scrollBox.top);
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

it('keeps a long compact section menu between its picker and the bottom bar', async () => {
  await page.viewport(320, 568);
  const cost = {
    id: 'cost',
    label: 'Cost',
    href: '/cost',
    icon,
    children: Array.from({ length: 14 }, (_, index) => ({
      id: `cost-${String(index + 1)}`,
      label: `Cost page ${String(index + 1)}`,
      href: `/cost/${String(index + 1)}`,
    })),
  };
  styles = document.createElement('style');
  styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} body{margin:0} *{box-sizing:border-box}`;
  document.head.append(styles);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(() => {
    root?.render(
      <>
        <AppNavigation
          {...labels}
          items={[cost]}
          pathname="/cost/1"
          profile={{ label: 'Account', initials: 'AB', href: '/profile' }}
        />
        <main style={{ padding: 8, paddingTop: 200 }}>
          <SectionPicker closeLabel="Close" item={cost} pathname="/cost/1" />
        </main>
      </>,
    );
    return Promise.resolve();
  });
  const trigger = page.getByRole('button', { name: 'Cost, Cost page 1' });
  await act(async () => trigger.click());
  const menu = page.getByRole('menu', { name: 'Cost' }).element();
  const panel = menu.closest('.bk-navigation-menu');
  if (panel === null) throw new Error('Menu panel did not render');
  const scroll = menu.querySelector<HTMLElement>('.bk-navigation-menu-items');
  if (scroll === null) throw new Error('Menu scroll container did not render');
  const close = page.getByRole('button', { name: 'Close', exact: true });
  const navigation = document.querySelector('.bk-navigation');
  if (navigation === null) throw new Error('Bottom navigation did not render');
  const expectBounds = () => {
    const menuBox = panel.getBoundingClientRect();
    expect(menuBox.top).toBeGreaterThanOrEqual(
      trigger.element().getBoundingClientRect().bottom + 8,
    );
    expect(menuBox.bottom).toBeLessThanOrEqual(navigation.getBoundingClientRect().top);
    expect(scroll.scrollHeight).toBeGreaterThan(scroll.clientHeight);
    expect(getComputedStyle(scroll).overflowY).toBe('auto');
    const closeBox = close.element().getBoundingClientRect();
    expect(closeBox.top).toBeGreaterThanOrEqual(scroll.getBoundingClientRect().bottom);
    expect(closeBox.bottom).toBeLessThanOrEqual(menuBox.bottom);
    expect(closeBox.height).toBeGreaterThanOrEqual(44);
  };
  expectBounds();
  await act(async () => userEvent.keyboard('{End}'));
  const last = page.getByRole('menuitem', { name: 'Cost page 14' }).element();
  expect(document.activeElement).toBe(last);
  expect(last.getBoundingClientRect().bottom).toBeLessThanOrEqual(
    menu.getBoundingClientRect().bottom,
  );
  expect(scroll.scrollTop).toBeGreaterThan(0);
  await act(async () => {
    await page.viewport(320, 480);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          resolve();
        });
      }),
    );
  });
  await expect
    .poll(() => panel.getBoundingClientRect().bottom <= navigation.getBoundingClientRect().top)
    .toBe(true);
  expectBounds();
  const main = host.querySelector('main');
  if (main === null) throw new Error('Main content did not render');
  await act(async () => {
    main.style.paddingTop = '240px';
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          resolve();
        });
      }),
    );
  });
  await expect
    .poll(() => panel.getBoundingClientRect().bottom <= navigation.getBoundingClientRect().top)
    .toBe(true);
  expectBounds();
  await expect.element(close).toBeVisible();
  await act(async () => {
    await close.click();
  });
  await expect.element(page.getByRole('menu')).not.toBeInTheDocument();
});

it('ellipsizes long labels inside all six compact targets at 320 pixels', async () => {
  await page.viewport(320, 568);
  styles = document.createElement('style');
  styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} body{margin:0}`;
  document.head.append(styles);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(() => {
    root?.render(
      <AppNavigation
        {...labels}
        items={Array.from({ length: 5 }, (_, index) => ({
          id: `section-${String(index)}`,
          label: `Arbeitsbereich mit langem Namen ${String(index)}`,
          href: `/section/${String(index)}`,
          icon,
        }))}
        pathname="/section/0"
        profile={{ label: 'Profil und persönliche Daten', initials: 'AB', href: '/profile' }}
      />,
    );
    return Promise.resolve();
  });
  const targets = host.querySelectorAll<HTMLElement>('.bk-navigation-item');
  expect(targets).toHaveLength(6);
  for (const target of targets) {
    const label = target.querySelector<HTMLElement>('.bk-navigation-label');
    if (label === null) throw new Error('Target label did not render');
    const box = target.getBoundingClientRect();
    const labelBox = label.getBoundingClientRect();
    expect(box.width).toBeGreaterThanOrEqual(44);
    expect(box.height).toBeGreaterThanOrEqual(44);
    expect(labelBox.left).toBeGreaterThanOrEqual(box.left);
    expect(labelBox.right).toBeLessThanOrEqual(box.right);
    expect(label.scrollWidth).toBeGreaterThan(label.clientWidth);
    expect(getComputedStyle(label).textOverflow).toBe('ellipsis');
    expect(getComputedStyle(label).overflow).toBe('hidden');
    expect(target.getAttribute('aria-label')).toBe(label.textContent);
  }
  expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(320);
});

it('keeps custom profile avatars decorative and preserves subtitles and menu focus', async () => {
  const errors: unknown[][] = [];
  vi.spyOn(console, 'error').mockImplementation((...args: unknown[]) => {
    errors.push(args);
  });
  vi.spyOn(console, 'warn').mockImplementation((...args: unknown[]) => {
    errors.push(args);
  });
  await page.viewport(1024, 720);
  styles = document.createElement('style');
  styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet}`;
  document.head.append(styles);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(() => {
    root?.render(
      <AppNavigation
        {...labels}
        items={items}
        pathname="/"
        width={1024}
        profile={{
          label: 'Account',
          subtitle: 'Ada, synced',
          initials: 'AD',
          renderAvatar: ({ size }) => (
            <span
              role="img"
              aria-label="Product glyph"
              tabIndex={0}
              data-testid="profile-glyph"
              style={{ width: size, height: size, border: '2px solid currentColor' }}
            >
              A
            </span>
          ),
          menu: [{ id: 'settings', label: 'Settings', href: '/settings' }],
        }}
      />,
    );
    return Promise.resolve();
  });
  const subtitle = page.getByText('Ada, synced', { exact: true });
  await expect.element(subtitle).toBeVisible();
  await act(async () => {
    await userEvent.click(page.getByRole('button', { name: 'Collapse navigation' }));
  });
  await expect.element(subtitle).not.toBeVisible();
  const trigger = page.getByRole('button', { name: 'Account, Ada, synced' });
  await expect.element(trigger).toBeVisible();
  await expect.element(page.getByTestId('profile-glyph')).toBeVisible();
  await expect.element(page.getByRole('img', { name: 'Product glyph' })).not.toBeInTheDocument();
  act(() => {
    trigger.element().focus();
    page.getByTestId('profile-glyph').element().focus();
  });
  await expect.element(trigger).toHaveFocus();
  const avatar = page.getByTestId('profile-glyph').element().parentElement;
  expect(avatar).not.toBeNull();
  if (avatar === null) throw new Error('Avatar wrapper is missing');
  expect(getComputedStyle(avatar).backgroundColor).toBe('rgba(0, 0, 0, 0)');
  await act(async () => {
    await userEvent.click(trigger);
  });
  await expect
    .element(
      page
        .getByRole('menu', { name: 'Account, Ada, synced' })
        .getByText('Ada, synced', { exact: true }),
    )
    .toBeVisible();
  await expect.element(page.getByRole('menuitem', { name: 'Settings' })).toHaveFocus();
  await act(async () => {
    await userEvent.keyboard('{Escape}');
  });
  await expect.element(trigger).toHaveFocus();
  expect(errors).toEqual([]);
});

it.each([600, 1024])('tabs into a visible Insights parent at width %s', async (width) => {
  await page.viewport(width, 720);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  const insights = {
    id: 'insights',
    label: 'Insights',
    href: '/insights',
    icon,
    children: [{ id: 'charts', label: 'Charts', href: '/insights/charts' }],
  };
  await act(() => {
    root?.render(
      <>
        <button type="button">Before navigation</button>
        <AppNavigation
          {...labels}
          width={width}
          defaultCollapsed
          items={[{ id: 'home', label: 'Home', href: '/', icon }, insights]}
          pathname="/insights/charts?range=4w"
        />
      </>,
    );
    return Promise.resolve();
  });
  page.getByRole('button', { name: 'Before navigation' }).element().focus();
  await act(async () => {
    await userEvent.keyboard('{Tab}');
  });
  if (width === 1024) {
    await expect.element(page.getByRole('button', { name: 'Expand navigation' })).toHaveFocus();
    await act(async () => {
      await userEvent.keyboard('{Tab}');
    });
  }
  await expect
    .element(page.getByRole(width === 1024 ? 'button' : 'link', { name: 'Insights' }))
    .toHaveFocus();
});

it('uses the body font in a standalone section picker', async () => {
  styles = document.createElement('style');
  styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} body { font-family: serif; }`;
  document.head.append(styles);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(() => {
    root?.render(<SectionPicker closeLabel="Close" item={section} pathname="/progress" />);
    return Promise.resolve();
  });
  const picker = host.querySelector('.bk-navigation-picker');
  expect(picker).not.toBeNull();
  if (picker === null) throw new Error('Section picker is missing');
  expect(getComputedStyle(picker).fontFamily).toBe(exampleTokens.typography.family.body);
});

function SlotFixture({
  brandLabel = 'Baukit',
  accessoryLabel = 'Workout active',
  dangerLabel = 'Delete account',
  closeLabel = 'Menü schließen',
}: {
  readonly brandLabel?: string;
  readonly accessoryLabel?: string;
  readonly dangerLabel?: string;
  readonly closeLabel?: string;
} = {}) {
  const [collapsed, setCollapsed] = useState(false);
  const [barHeight, setBarHeight] = useState(64);
  return (
    <>
      <AppNavigation
        {...labels}
        closeLabel={closeLabel}
        items={items}
        pathname="/delete"
        collapsed={collapsed}
        onCollapsedChange={setCollapsed}
        onBarHeightChange={setBarHeight}
        renderBrand={({ collapsed: small }) => <strong>{small ? 'B' : brandLabel}</strong>}
        renderAccessory={({ collapsed: small }) => (
          <span role="status" aria-label="Workout status">
            {small ? '●' : accessoryLabel}
          </span>
        )}
        profile={{
          label: 'Account',
          initials: 'AB',
          menu: [
            { id: 'settings', label: 'Settings', href: '/settings' },
            { id: 'delete', label: dangerLabel, tone: 'danger', href: '/delete' },
            { id: 'signout', label: 'Sign out', tone: 'danger', onSelect: () => undefined },
          ],
        }}
      />
      <main
        style={{
          marginLeft: window.innerWidth >= 1024 ? (collapsed ? 76 : 280) : 0,
          paddingBottom: barHeight,
        }}
      >
        <h1 style={{ overflowWrap: 'anywhere' }}>Navigation with slots</h1>
        <SectionPicker closeLabel="Auswahl schließen" item={section} pathname="/progress" />
      </main>
    </>
  );
}
it.each(
  [1, 2].flatMap((scale) =>
    [320, 1024].flatMap((width) => ['light', 'dark'].map((theme) => ({ scale, width, theme }))),
  ),
)(
  'keeps slots and danger menus accessible at $width, font scale $scale, $theme',
  async ({ scale, width, theme }) => {
    await page.viewport(width, 900);
    document.documentElement.lang = 'de';
    document.documentElement.dataset['theme'] = theme;
    styles = document.createElement('style');
    styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} html{font-size:${String(16 * scale)}px} body{margin:0;background:var(--bk-color-background-primary);color:var(--bk-color-text-primary)}`;
    document.head.append(styles);
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    await act(() => {
      root?.render(<SlotFixture />);
      return Promise.resolve();
    });
    const navigation = page.getByTestId('primary-navigation').element();
    const slots = navigation.querySelector('.bk-navigation-slots');
    if (slots === null) throw new Error('Slots are missing');
    expect(
      page.getByText('Baukit', { exact: true }).element().getBoundingClientRect().width,
    ).toBeGreaterThan(0);
    const brandLines = document.createRange();
    brandLines.selectNodeContents(page.getByText('Baukit', { exact: true }).element());
    expect(brandLines.getClientRects()).toHaveLength(1);
    expect(page.getByRole('status', { name: 'Workout status' }).element().textContent).toBe(
      'Workout active',
    );
    const home = page.getByRole('link', { name: 'Home' }).element();
    expect(home.getBoundingClientRect().top).toBeGreaterThanOrEqual(
      slots.getBoundingClientRect().bottom,
    );
    for (const slot of slots.children) {
      const bounds = slot.getBoundingClientRect();
      expect(bounds.left).toBeGreaterThanOrEqual(navigation.getBoundingClientRect().left);
      expect(bounds.right).toBeLessThanOrEqual(navigation.getBoundingClientRect().right);
    }
    if (width === 320) {
      const main = host.querySelector('main');
      if (main === null) throw new Error('Content is missing');
      await expect
        .poll(() => Number.parseFloat(getComputedStyle(main).paddingBottom))
        .toBe(navigation.getBoundingClientRect().height);
      const iconBox = home.querySelector('.bk-navigation-icon')?.getBoundingClientRect();
      const labelBox = home.querySelector('.bk-navigation-label')?.getBoundingClientRect();
      if (iconBox === undefined || labelBox === undefined)
        throw new Error('Icon or label is missing');
      expect(labelBox.top).toBeGreaterThanOrEqual(iconBox.bottom);
      expect(navigation.getBoundingClientRect().height).toBeGreaterThan(64);
    } else {
      await act(async () => {
        await page.getByRole('button', { name: 'Collapse navigation' }).click();
      });
      await expect.element(page.getByText('B', { exact: true })).toBeVisible();
      expect(page.getByRole('status', { name: 'Workout status' }).element().textContent).toBe('●');
    }
    const account = page.getByRole('button', { name: 'Account' });
    await act(async () => {
      await account.click();
    });
    const danger = page.getByRole('menuitem', { name: 'Delete account' }).element();
    expect(getComputedStyle(danger).fontSize).toBe(`${String(16 * scale)}px`);
    const expectedColor = exampleTokens.color.status.danger[theme === 'dark' ? 'dark' : 'light'];
    const probe = document.createElement('span');
    probe.style.color = expectedColor;
    host.append(probe);
    expect(getComputedStyle(danger).color).toBe(getComputedStyle(probe).color);
    probe.remove();
    expect((await axe.run(host)).violations).toEqual([]);
    await act(async () => {
      await userEvent.keyboard('{End}');
    });
    await expect.element(page.getByRole('menuitem', { name: 'Sign out' })).toHaveFocus();
    await act(async () => {
      await page.getByRole('button', { name: 'Menü schließen' }).click();
    });
    await expect.element(account).toHaveFocus();
    await expect.element(page.getByRole('menu')).not.toBeInTheDocument();
    expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(width);
  },
);

it.each([320, 1024])('wraps long slot and menu text at $0 px and font scale 2', async (width) => {
  await page.viewport(width, 900);
  document.documentElement.lang = 'de';
  styles = document.createElement('style');
  styles.textContent = `${toCssVariables(exampleTokens)} ${stylesheet} html{font-size:32px} body{margin:0}`;
  document.head.append(styles);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  const dangerLabel = 'Kontozugriffdauerhaftbeenden';
  const closeLabel = 'MenüschließenundzurÜbersichtzurückkehren';
  await act(() => {
    root?.render(
      <SlotFixture
        brandLabel="BaukitTrainingsverwaltung"
        accessoryLabel="Kraftausdauertrainingläuft"
        dangerLabel={dangerLabel}
        closeLabel={closeLabel}
      />,
    );
    return Promise.resolve();
  });
  const navigation = page.getByTestId('primary-navigation').element();
  const slots = navigation.querySelector('.bk-navigation-slots');
  if (slots === null) throw new Error('Slots are missing');
  for (const slot of slots.children) {
    const bounds = slot.getBoundingClientRect();
    expect(bounds.left).toBeGreaterThanOrEqual(navigation.getBoundingClientRect().left);
    expect(bounds.right).toBeLessThanOrEqual(navigation.getBoundingClientRect().right);
    expect(slot.scrollWidth).toBeLessThanOrEqual(slot.clientWidth);
    const text = document.createRange();
    text.selectNodeContents(slot);
    expect(text.getClientRects().length).toBeGreaterThan(1);
    for (const line of text.getClientRects()) {
      expect(line.left).toBeGreaterThanOrEqual(bounds.left);
      expect(line.right).toBeLessThanOrEqual(bounds.right);
    }
  }
  if (width === 320) {
    const main = host.querySelector('main');
    if (main === null) throw new Error('Content is missing');
    await expect
      .poll(() => Number.parseFloat(getComputedStyle(main).paddingBottom))
      .toBe(navigation.getBoundingClientRect().height);
    expect(
      page.getByRole('link', { name: 'Home' }).element().getBoundingClientRect().top,
    ).toBeGreaterThanOrEqual(slots.getBoundingClientRect().bottom);
  }
  await act(async () => {
    await page.getByRole('button', { name: 'Account' }).click();
  });
  const danger = page.getByRole('menuitem', { name: dangerLabel }).element();
  const close = page.getByRole('button', { name: closeLabel }).element();
  for (const control of [danger, close]) {
    expect(control.scrollWidth).toBeLessThanOrEqual(control.clientWidth);
    const text = document.createRange();
    text.selectNodeContents(control);
    expect(text.getClientRects().length).toBeGreaterThan(1);
    const bounds = control.getBoundingClientRect();
    for (const line of text.getClientRects()) {
      expect(line.left).toBeGreaterThanOrEqual(bounds.left);
      expect(line.right).toBeLessThanOrEqual(bounds.right);
    }
  }
  expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(width);
});

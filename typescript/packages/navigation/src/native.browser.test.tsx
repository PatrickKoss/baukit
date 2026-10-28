import { afterEach, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';
import { Platform, Text, View } from 'react-native';
import { AppNavigation, SectionPicker, type NavigationTheme } from './native.js';

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const theme: NavigationTheme = {
  background: '#fff',
  text: '#111',
  muted: '#595959',
  danger: '#b3261e',
  activeBackground: '#dbe9f8',
  activeText: '#005fcc',
  ancestorText: '#004799',
  border: '#767676',
  focus: '#005fcc',
  radius: 4,
  spacing: 8,
};
const icon = () => <Text>●</Text>;
let root: Root | undefined;
let host: HTMLDivElement | undefined;

afterEach(async () => {
  await act(() => {
    root?.unmount();
    return Promise.resolve();
  });
  host?.remove();
});
async function mount(children: React.ReactNode, width = 320) {
  await page.viewport(width, 568);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(() => {
    root?.render(children);
    return Promise.resolve();
  });
}
async function click(target: Parameters<typeof userEvent.click>[0]) {
  await act(async () => {
    await userEvent.click(target);
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          resolve();
        }),
      );
    });
  });
}
function expectVisibleMenu() {
  const menu = page.getByRole('menu').element();
  const rect = menu.getBoundingClientRect();
  expect(rect.width).toBeGreaterThan(200);
  expect(rect.left).toBeGreaterThanOrEqual(0);
  expect(rect.right).toBeLessThanOrEqual(320);
  expect(rect.top).toBeGreaterThanOrEqual(0);
  expect(rect.bottom).toBeLessThanOrEqual(568);
  expect(host?.contains(menu)).toBe(false);
  for (const entry of menu.querySelectorAll('[role="menuitem"], [role="button"]')) {
    const bounds = entry.getBoundingClientRect();
    const hit = document.elementFromPoint(
      bounds.left + bounds.width / 2,
      bounds.top + bounds.height / 2,
    );
    expect(hit !== null && entry.contains(hit)).toBe(true);
  }
}
it.each(
  [320, 360].flatMap((width) =>
    (['ios', 'android'] as const).map((platform) => ({ width, platform })),
  ),
)(
  'keeps six items and profile apart at $width px with $platform target sizing',
  async ({ width, platform }) => {
    const originalPlatform = Platform.OS;
    Platform.OS = platform;
    try {
      await mount(
        <View style={{ width, height: 568, overflow: 'hidden', justifyContent: 'flex-end' }}>
          <AppNavigation
            items={Array.from({ length: 6 }, (_, index) => ({
              id: `page-${String(index)}`,
              label: `Page ${String(index)}`,
              href: `/page/${String(index)}`,
              icon,
            }))}
            label="Primary"
            collapseLabel="Collapse"
            expandLabel="Expand"
            closeLabel="Close"
            width={width}
            pathname="/page/0"
            theme={theme}
            onNavigate={vi.fn()}
            profile={{ label: 'Account', initials: 'AB', href: '/profile' }}
          />
        </View>,
        width,
      );
      const navigation = page.getByTestId('primary-navigation').element();
      const scroll = page.getByTestId('navigation-items').element();
      const targets = navigation.querySelectorAll<HTMLElement>('[role="link"]');
      expect(targets).toHaveLength(7);
      const minimumTarget = platform === 'android' ? 48 : 44;
      let previousRight = scroll.getBoundingClientRect().left;
      for (const target of targets) {
        const box = target.getBoundingClientRect();
        expect(box.width).toBeGreaterThanOrEqual(minimumTarget);
        expect(box.height).toBeGreaterThanOrEqual(minimumTarget);
        expect(box.left).toBeGreaterThanOrEqual(previousRight);
        previousRight = box.right;
      }
      expect(navigation.getBoundingClientRect().width).toBe(width);
      if (platform === 'android' && width === 320)
        expect(scroll.scrollWidth).toBeGreaterThan(scroll.clientWidth);
      scroll.scrollLeft = scroll.scrollWidth;
      const profile = page.getByRole('link', { name: 'Account' }).element().getBoundingClientRect();
      const scrollBox = scroll.getBoundingClientRect();
      expect(profile.left).toBeGreaterThanOrEqual(scrollBox.left);
      expect(profile.right).toBeLessThanOrEqual(scrollBox.right);
      for (const target of Array.from(targets).slice(0, 6))
        expect(target.getBoundingClientRect().right).toBeLessThanOrEqual(profile.left);
      expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(width);
    } finally {
      Platform.OS = originalPlatform;
    }
  },
);

it('presents an unclipped compact profile menu at 320 px', async () => {
  const navigate = vi.fn();
  await mount(
    <View style={{ height: 568, overflow: 'hidden', justifyContent: 'flex-end' }}>
      <AppNavigation
        items={[{ id: 'home', label: 'Home', href: '/', icon }]}
        label="Primary"
        collapseLabel="Collapse"
        expandLabel="Expand"
        closeLabel="Close"
        width={320}
        pathname="/settings/profile"
        theme={theme}
        onNavigate={navigate}
        profile={{
          label: 'Account',
          initials: 'AB',
          menu: [
            { id: 'settings', label: 'App settings', href: '/settings' },
            { id: 'profile', label: 'Profile and data', href: '/settings/profile' },
          ],
        }}
      />
    </View>,
  );
  await click(page.getByRole('button', { name: 'Account' }));
  await expect.element(page.getByRole('menu')).toBeInTheDocument();
  expectVisibleMenu();
  await expect.element(page.getByRole('menuitem', { name: 'Profile and data' })).toHaveFocus();
  await click(page.getByRole('menuitem', { name: 'App settings' }));
  expect(navigate).toHaveBeenCalledWith('/settings');
  await expect.element(page.getByRole('button', { name: 'Account' })).toHaveFocus();
});
it.each([false, true])('focuses a picker child at 320 px with replace=%s', async (replace) => {
  const navigate = vi.fn();
  await mount(
    <View style={{ height: 568, overflow: 'hidden' }}>
      <SectionPicker
        replace={replace}
        closeLabel="Close"
        theme={theme}
        pathname="/progress/history"
        onNavigate={navigate}
        item={{
          id: 'progress',
          label: 'Progress',
          href: '/progress',
          icon,
          children: [
            { id: 'overview', label: 'Overview', href: '/progress' },
            { id: 'history', label: 'History', href: '/progress/history' },
          ],
        }}
      />
    </View>,
  );
  await click(page.getByRole('button', { name: 'Progress, History' }));
  await expect.element(page.getByRole('menu')).toBeInTheDocument();
  expectVisibleMenu();
  await expect.element(page.getByRole('menuitem', { name: 'History' })).toHaveFocus();
  await click(page.getByRole('menuitem', { name: 'Overview' }));
  if (replace) expect(navigate).toHaveBeenCalledWith('/progress', { replace: true });
  else expect(navigate).toHaveBeenCalledWith('/progress');
  await expect.element(page.getByRole('button', { name: 'Progress, History' })).toHaveFocus();
});

it('keeps a custom native avatar decorative with subtitle, typography and menu focus', async () => {
  await mount(
    <AppNavigation
      label="Primary"
      collapseLabel="Collapse"
      expandLabel="Expand"
      closeLabel="Close"
      items={[{ id: 'home', label: 'Home', href: '/', icon }]}
      width={320}
      pathname="/"
      theme={{ ...theme, typography: { fontFamily: 'monospace', fontSize: 18, barFontSize: 12 } }}
      onNavigate={vi.fn()}
      profile={{
        label: 'Account',
        subtitle: 'Ada, synced',
        initials: 'AD',
        renderAvatar: ({ size }) => (
          <Text
            accessibilityRole="image"
            accessibilityLabel="Product glyph"
            testID="profile-glyph"
            style={{ width: size, height: size }}
          >
            A
          </Text>
        ),
        menu: [{ id: 'settings', label: 'Settings', href: '/settings' }],
      }}
    />,
  );
  const trigger = page.getByRole('button', { name: 'Account, Ada, synced' });
  await expect.element(trigger).toBeVisible();
  await expect.element(page.getByTestId('profile-glyph')).toBeVisible();
  await expect.element(page.getByRole('img', { name: 'Product glyph' })).not.toBeInTheDocument();
  await expect.element(page.getByText('Ada, synced', { exact: true })).not.toBeInTheDocument();
  await click(trigger);
  const menu = page.getByRole('menu', { name: 'Account, Ada, synced' });
  await expect.element(menu.getByText('Ada, synced', { exact: true })).toBeVisible();
  const settings = document.querySelector<HTMLElement>('[role="menuitem"]');
  expect(settings).not.toBeNull();
  const text = settings?.querySelector<HTMLElement>('[dir="auto"]');
  expect(text).not.toBeNull();
  if (text === null || text === undefined) throw new Error('Settings text is missing');
  expect(getComputedStyle(text).fontFamily).toContain('monospace');
  expect(getComputedStyle(text).fontSize).toBe('18px');
  await expect.element(page.getByRole('menuitem', { name: 'Settings' })).toHaveFocus();
  await click(page.getByRole('button', { name: 'Close' }));
  await expect.element(trigger).toHaveFocus();
});

it('preserves the compact brand width when accessory text wraps', async () => {
  const heights = vi.fn<(height: number) => void>();
  await mount(
    <View style={{ height: 568, justifyContent: 'flex-end' }}>
      <AppNavigation
        label="Primary"
        collapseLabel="Collapse"
        expandLabel="Expand"
        closeLabel="Close"
        items={[{ id: 'home', label: 'Home', href: '/', icon }]}
        width={320}
        pathname="/"
        theme={theme}
        onNavigate={vi.fn()}
        onBarHeightChange={heights}
        renderBrand={() => <Text style={{ fontSize: 32 }}>Baukit</Text>}
        renderAccessory={() => <Text style={{ fontSize: 32 }}>Workout active</Text>}
      />
    </View>,
  );
  const navigation = page.getByTestId('primary-navigation').element();
  await act(async () => {
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          resolve();
        }),
      );
    });
  });
  await expect
    .poll(() => heights.mock.lastCall?.[0])
    .toBe(navigation.getBoundingClientRect().height);
  const brand = page.getByText('Baukit', { exact: true }).element();
  const brandLines = document.createRange();
  brandLines.selectNodeContents(brand);
  expect(brandLines.getClientRects()).toHaveLength(1);
  const accessory = page.getByTestId('navigation-accessory').element().getBoundingClientRect();
  expect(accessory.left).toBeGreaterThanOrEqual(brand.getBoundingClientRect().right);
  expect(accessory.right).toBeLessThanOrEqual(navigation.getBoundingClientRect().right);
  expect(navigation.getBoundingClientRect().height).toBeGreaterThan(75);
});

it('rings a web navigation target for keyboard focus but not after a pointer click', async () => {
  await mount(
    <View style={{ width: 320, height: 568, justifyContent: 'flex-end' }}>
      <AppNavigation
        items={['Home', 'Inbox'].map((label) => ({
          id: label,
          label,
          href: `/${label.toLowerCase()}`,
          icon,
        }))}
        label="Primary"
        collapseLabel="Collapse"
        expandLabel="Expand"
        closeLabel="Close"
        width={320}
        pathname="/home"
        theme={theme}
        onNavigate={vi.fn()}
      />
    </View>,
  );
  const inbox = page.getByRole('link', { name: 'Inbox' }).element() as HTMLElement;
  const resting = getComputedStyle(inbox).borderTopColor;
  await click(inbox);
  expect(inbox.matches(':focus-visible')).toBe(false);
  expect(getComputedStyle(inbox).borderTopColor).toBe(resting);
  await act(async () => {
    await userEvent.tab();
    inbox.focus();
  });
  expect(inbox.matches(':focus-visible')).toBe(true);
  expect(getComputedStyle(inbox).borderTopColor).toBe('rgb(0, 95, 204)');
});

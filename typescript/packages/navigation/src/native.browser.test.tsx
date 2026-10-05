import { afterEach, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';
import { Text, View } from 'react-native';
import { AppNavigation, SectionPicker, type NavigationTheme } from './native.js';

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const theme: NavigationTheme = {
  background: '#fff',
  text: '#111',
  muted: '#595959',
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
async function mount(children: React.ReactNode) {
  await page.viewport(320, 568);
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
it('presents an unclipped section picker and focuses its selected child at 320 px', async () => {
  const navigate = vi.fn();
  await mount(
    <View style={{ height: 568, overflow: 'hidden' }}>
      <SectionPicker
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
  expect(navigate).toHaveBeenCalledWith('/progress');
  await expect.element(page.getByRole('button', { name: 'Progress, History' })).toHaveFocus();
});

it('renders a compact profile subtitle in the menu and uses the supplied font family', async () => {
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
        menu: [{ id: 'settings', label: 'Settings', href: '/settings' }],
      }}
    />,
  );
  const trigger = page.getByRole('button', { name: 'Account, Ada, synced' });
  await expect.element(trigger).toBeVisible();
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
});

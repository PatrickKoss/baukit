import { fireEvent, render, screen, within } from '@testing-library/react-native';
import { AccessibilityInfo, Platform, Text } from 'react-native';
import {
  AppNavigation,
  SectionPicker,
  type NavigationIcon,
  type NavigationTheme,
} from './native.js';
import type { NavigationItem } from './index.js';

// Resolve lazy native modules during setup, before Jest starts each test's timer.
const nativeModules = jest.requireActual<typeof import('react-native')>('react-native');
for (const name of [
  'AccessibilityInfo',
  'Image',
  'Modal',
  'Pressable',
  'ScrollView',
  'Text',
  'View',
  'useWindowDimensions',
]) {
  Reflect.get(nativeModules, name);
}

const labels = {
  label: 'Primary',
  collapseLabel: 'Collapse navigation',
  expandLabel: 'Expand navigation',
  closeLabel: 'Close',
};

const icon: NavigationIcon = ({ active }) => <Text>{active ? 'filled' : 'outline'}</Text>;
const section: NavigationItem<NavigationIcon> = {
  id: 'progress',
  label: 'Progress',
  href: '/progress',
  icon,
  children: [
    { id: 'overview', label: 'Overview', href: '/progress', icon },
    { id: 'history', label: 'History', href: '/progress/history', icon },
  ],
};
const items = [{ id: 'home', label: 'Home', href: '/', icon }, section];
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
const profile = { label: 'Account', initials: 'AB', href: '/profile' };
const event = () => ({ nativeEvent: {}, preventDefault: jest.fn() });

it('keeps keyboard focus visible on active navigation, picker and menu entries', async () => {
  await render(
    <>
      <AppNavigation
        {...labels}
        items={items}
        width={1024}
        pathname="/progress/history"
        theme={theme}
        onNavigate={jest.fn()}
      />
      <SectionPicker
        closeLabel={labels.closeLabel}
        item={section}
        pathname="/progress/history"
        theme={theme}
        onNavigate={jest.fn()}
      />
    </>,
  );
  const active = screen.getByRole('link', { name: 'History' });
  await fireEvent(active, 'focus', event());
  expect(active).toHaveStyle({
    borderWidth: 3,
    borderColor: theme.activeText,
    backgroundColor: theme.activeBackground,
  });
  await fireEvent(active, 'blur', event());
  expect(active).toHaveStyle({ borderColor: 'transparent' });
  const picker = screen.getByRole('button', { name: 'Progress, History' });
  await fireEvent(picker, 'focus', event());
  expect(picker).toHaveStyle({ borderWidth: 3, borderColor: theme.focus });
  await fireEvent(picker, 'blur', event());
  expect(picker).toHaveStyle({ borderColor: theme.border });
  await fireEvent.press(picker, event());
  const entry = screen.getByRole('menuitem', { name: 'History' });
  await fireEvent(entry, 'focus', event());
  expect(entry).toHaveStyle({ borderWidth: 3, borderColor: theme.activeText });
  await fireEvent(entry, 'blur', event());
  expect(entry).toHaveStyle({ borderColor: 'transparent' });
});

it('shows an active disclosure with its selected child directly inside its section', async () => {
  const navigate = jest.fn();
  await render(
    <AppNavigation
      {...labels}
      items={items}
      profile={profile}
      width={1024}
      pathname="/progress/history"
      theme={theme}
      onNavigate={navigate}
    />,
  );
  expect(screen.getByRole('button', { name: 'Progress' })).toHaveProp('accessibilityState', {
    expanded: true,
    selected: true,
  });
  expect(screen.getByRole('link', { name: 'History' })).toHaveProp('accessibilityState', {
    selected: true,
  });
  expect(screen.getByRole('button', { name: 'Progress' })).toHaveStyle({
    backgroundColor: theme.background,
  });
  expect(
    within(screen.getByRole('button', { name: 'Progress' })).getByText('Progress'),
  ).toHaveStyle({
    color: theme.ancestorText,
    fontWeight: '400',
  });
  expect(screen.getByRole('link', { name: 'History' })).toHaveStyle({
    backgroundColor: theme.activeBackground,
  });
  expect(within(screen.getByRole('link', { name: 'History' })).getByText('History')).toHaveStyle({
    color: theme.activeText,
    fontWeight: '700',
  });
  for (const target of [
    screen.getByRole('button', { name: 'Progress' }),
    screen.getByRole('link', { name: 'History' }),
  ]) {
    expect(within(target).getByText('filled', { includeHiddenElements: true })).toBeOnTheScreen();
  }
  expect([
    within(screen.getByTestId('navigation-section-progress')).getByRole('link', {
      name: 'History',
    }),
  ]).toContain(screen.getByRole('link', { name: 'History' }));
  await fireEvent.press(screen.getByRole('button', { name: 'Progress' }), event());
  expect(screen.queryByRole('link', { name: 'History' })).toBeNull();
  expect(navigate).not.toHaveBeenCalled();
  await fireEvent.press(screen.getByRole('link', { name: 'Home' }), event());
  expect(navigate).toHaveBeenCalledWith('/');
});
it('expands a collapsed rail, opens its group and notifies the caller', async () => {
  const changed = jest.fn();
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={1024}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
      defaultCollapsed
      onCollapsedChange={changed}
    />,
  );
  expect(screen.queryByText('Progress')).toBeNull();
  await fireEvent.press(screen.getByRole('button', { name: 'Progress' }), event());
  expect(changed).toHaveBeenCalledWith(false);
  expect(screen.getByRole('button', { name: 'Progress' })).toHaveProp('accessibilityState', {
    expanded: true,
    selected: false,
  });
  expect(screen.getByRole('link', { name: 'History' })).toBeOnTheScreen();
  await fireEvent.press(screen.getByRole('button', { name: 'Collapse navigation' }), event());
  expect(changed).toHaveBeenLastCalledWith(true);
  expect(screen.queryByText('Progress')).toBeNull();
});
it('fills the active parent only while the rail is collapsed', async () => {
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={1024}
      pathname="/progress/history"
      theme={theme}
      onNavigate={jest.fn()}
      defaultCollapsed
    />,
  );
  const parent = screen.getByRole('button', { name: 'Progress' });
  expect(parent).toHaveStyle({ backgroundColor: theme.activeBackground });
  expect(screen.queryByRole('link', { name: 'History' })).toBeNull();
  await fireEvent.press(parent, event());
  expect(parent).toHaveStyle({ backgroundColor: theme.background });
  expect(screen.getByRole('link', { name: 'History' })).toHaveStyle({
    backgroundColor: theme.activeBackground,
  });
});
it('fills an active main item without children', async () => {
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={1024}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
    />,
  );
  const home = screen.getByRole('link', { name: 'Home' });
  expect(home).toHaveStyle({ backgroundColor: theme.activeBackground, borderColor: 'transparent' });
  expect(within(home).getByText('Home')).toHaveStyle({
    color: theme.activeText,
    fontWeight: '700',
  });
});
it('respects controlled collapse and route changes', async () => {
  const changed = jest.fn();
  const props = { items, width: 1024, theme, onNavigate: jest.fn(), onCollapsedChange: changed };
  const view = await render(<AppNavigation {...labels} {...props} collapsed pathname="/" />);
  await fireEvent.press(screen.getByRole('button', { name: 'Progress' }), event());
  expect(changed).toHaveBeenCalledWith(false);
  expect(screen.queryByText('Progress')).toBeNull();
  await view.rerender(
    <AppNavigation {...labels} {...props} collapsed={false} pathname="/progress/history" />,
  );
  expect(screen.getByRole('link', { name: 'History' })).toHaveProp('accessibilityState', {
    selected: true,
  });
});
it.each([320, 1023])(
  'rotates active bottom tabs and navigates inactive tabs to the root at %i',
  async (width) => {
    const navigate = jest.fn();
    const view = await render(
      <AppNavigation
        {...labels}
        items={items}
        width={width}
        pathname="/progress"
        theme={theme}
        onNavigate={navigate}
      />,
    );
    expect(screen.getByRole('link', { name: 'Progress' })).toHaveStyle({
      backgroundColor: theme.activeBackground,
    });
    await fireEvent.press(screen.getByRole('link', { name: 'Progress' }), event());
    expect(navigate).toHaveBeenLastCalledWith('/progress/history');
    await view.rerender(
      <AppNavigation
        {...labels}
        items={items}
        width={width}
        pathname="/progress/history"
        theme={theme}
        onNavigate={navigate}
      />,
    );
    await fireEvent.press(screen.getByRole('link', { name: 'Progress' }), event());
    expect(navigate).toHaveBeenLastCalledWith('/progress');
    await fireEvent.press(screen.getByRole('link', { name: 'Home' }), event());
    expect(navigate).toHaveBeenLastCalledWith('/');
  },
);
it.each([320, 1024])('keeps profile last at %i', async (width) => {
  await render(
    <AppNavigation
      {...labels}
      items={items}
      profile={profile}
      width={width}
      pathname="/profile"
      theme={theme}
      onNavigate={jest.fn()}
    />,
  );
  const links = screen.getAllByRole('link');
  expect(links.at(-1)).toBe(screen.getByRole('link', { name: 'Account' }));
  expect(links.at(-1)).toHaveProp('accessibilityState', { selected: true });
  expect(links.at(-1)).toHaveStyle({ backgroundColor: theme.activeBackground });
});
it.each([
  { metaKey: true },
  { ctrlKey: true },
  { shiftKey: true },
  { altKey: true },
  { button: 1 },
])('preserves modified web presses: %j', async (modifier) => {
  const navigate = jest.fn();
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      theme={theme}
      onNavigate={navigate}
    />,
  );
  const press = { nativeEvent: modifier, preventDefault: jest.fn() };
  await fireEvent.press(screen.getByRole('link', { name: 'Home' }), press);
  expect(navigate).not.toHaveBeenCalled();
  expect(press.preventDefault).not.toHaveBeenCalled();
});
it('uses roving tab stops for Arrow, Home and End', async () => {
  await render(
    <AppNavigation
      {...labels}
      items={items}
      profile={profile}
      width={1024}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
    />,
  );
  const key = (value: string) => ({ nativeEvent: { key: value }, preventDefault: jest.fn() });
  expect(screen.getByRole('link', { name: 'Home' })).toHaveProp('tabIndex', 0);
  await fireEvent(screen.getByRole('link', { name: 'Home' }), 'keyDown', key('ArrowDown'));
  expect(screen.getByRole('button', { name: 'Progress' })).toHaveProp('tabIndex', 0);
  await fireEvent(screen.getByRole('button', { name: 'Progress' }), 'keyDown', key('End'));
  expect(screen.getByRole('link', { name: 'Account' })).toHaveProp('tabIndex', 0);
  await fireEvent(screen.getByRole('link', { name: 'Account' }), 'keyDown', key('Home'));
  expect(screen.getByRole('link', { name: 'Home' })).toHaveProp('tabIndex', 0);
});
it('opens a profile menu, runs an entry and closes', async () => {
  const action = jest.fn();
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [{ id: 'signout', label: 'Sign out', onSelect: action }],
      }}
    />,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Account' }), event());
  expect(screen.getByRole('menuitem', { name: 'Sign out' })).toBeOnTheScreen();
  await fireEvent.press(screen.getByRole('menuitem', { name: 'Sign out' }), event());
  expect(action).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('menuitem')).toBeNull();
});
it('shows a full-width section picker and navigates directly to a subpage', async () => {
  const navigate = jest.fn();
  await render(
    <SectionPicker
      closeLabel={labels.closeLabel}
      item={section}
      pathname="/progress"
      theme={theme}
      onNavigate={navigate}
    />,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Progress, Overview' }), event());
  await fireEvent.press(screen.getByRole('menuitem', { name: 'History' }), event());
  expect(navigate).toHaveBeenCalledWith('/progress/history');
  expect(screen.queryByRole('menuitem')).toBeNull();
});
it('skips disabled menu entries with the keyboard and dismisses with Escape', async () => {
  const action = jest.fn();
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [
          { id: 'settings', label: 'Settings', href: '/settings' },
          { id: 'disabled', label: 'Unavailable', href: '/unavailable', disabled: true },
          { id: 'signout', label: 'Sign out', onSelect: action },
        ],
      }}
    />,
  );
  const key = (value: string) => ({
    nativeEvent: { key: value },
    preventDefault: jest.fn(),
    stopPropagation: jest.fn(),
  });
  await fireEvent.press(screen.getByRole('button', { name: 'Account' }), event());
  expect(screen.getByRole('menuitem', { name: 'Unavailable' })).toBeDisabled();
  await fireEvent(screen.getByRole('menuitem', { name: 'Settings' }), 'keyDown', key('ArrowDown'));
  expect(screen.getByRole('menuitem', { name: 'Sign out' })).toHaveProp('tabIndex', 0);
  await fireEvent(screen.getByTestId('navigation-menu'), 'keyDown', key('Escape'));
  expect(screen.queryByTestId('navigation-menu')).toBeNull();
  expect(action).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'Account' })).toHaveProp('accessibilityState', {
    selected: false,
    expanded: false,
  });
});

it('marks the current section picker entry selected and highlights it', async () => {
  await render(
    <SectionPicker
      closeLabel="Schließen"
      item={section}
      pathname="/progress/history?tab=recent"
      theme={theme}
      onNavigate={jest.fn()}
    />,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Progress, History' }), event());
  const current = screen.getByRole('menuitem', { name: 'History', selected: true });
  expect(current).toHaveStyle({ backgroundColor: theme.activeBackground });
  expect(within(current).getByText('History')).toHaveStyle({
    color: theme.activeText,
    fontWeight: '700',
  });
  expect(screen.getByRole('menuitem', { name: 'Overview' })).toHaveProp('accessibilityState', {
    disabled: false,
    selected: false,
  });
  const close = screen.getByRole('button', { name: 'Schließen' });
  expect(within(close).getByText('Schließen')).toBeOnTheScreen();
  await fireEvent.press(close, event());
  expect(screen.queryByRole('menuitem')).toBeNull();
});
it('marks only the most-specific profile entry selected and honors custom matches', async () => {
  const props = {
    ...labels,
    items,
    width: 320,
    theme,
    onNavigate: jest.fn(),
    profile: {
      label: 'Account',
      initials: 'AB',
      menu: [
        { id: 'settings', label: 'App settings', href: '/settings' },
        {
          id: 'profile-data',
          label: 'Profile and data',
          href: '/settings/profile?tab=data',
          matches: (path: string) =>
            path.startsWith('/alias') || path.startsWith('/settings/profile'),
        },
      ],
    },
  };
  const { rerender } = await render(<AppNavigation {...props} pathname="/settings/profile" />);
  await fireEvent.press(screen.getByRole('button', { name: 'Account' }), event());
  const current = screen.getByRole('menuitem', { name: 'Profile and data', selected: true });
  expect(current).toHaveStyle({ backgroundColor: theme.activeBackground });
  expect(within(current).getByText('Profile and data')).toHaveStyle({
    color: theme.activeText,
    fontWeight: '700',
  });
  expect(screen.getByRole('menuitem', { name: 'App settings' })).toHaveProp('accessibilityState', {
    disabled: false,
    selected: false,
  });
  await rerender(<AppNavigation {...props} pathname="/alias" />);
  expect(
    screen.getByRole('menuitem', { name: 'Profile and data', selected: true }),
  ).toBeOnTheScreen();
});
it.each(['android', 'ios'] as const)(
  'sizes every %s target to its platform minimum',
  async (platform) => {
    const original = Platform.OS;
    Platform.OS = platform;
    try {
      const minimum = platform === 'android' ? 48 : 44;
      const props = {
        ...labels,
        items,
        theme,
        onNavigate: jest.fn(),
        profile: {
          label: 'Account',
          initials: 'AB',
          menu: [
            { id: 'settings', label: 'Settings', href: '/settings' },
            { id: 'action', label: 'Action', onSelect: jest.fn() },
          ],
        },
      };
      const { rerender } = await render(
        <>
          <AppNavigation {...props} width={1024} pathname="/progress/history" />
          <SectionPicker
            closeLabel={labels.closeLabel}
            item={section}
            pathname="/progress/history"
            theme={theme}
            onNavigate={jest.fn()}
          />
        </>,
      );
      function expectTargets() {
        for (const role of ['button', 'link', 'menuitem'] as const) {
          for (const target of screen.queryAllByRole(role))
            expect(target).toHaveStyle({ minWidth: minimum, minHeight: minimum });
        }
      }
      expectTargets();
      await fireEvent.press(screen.getByRole('button', { name: 'Account' }), event());
      expect(screen.getAllByRole('menuitem')).toHaveLength(2);
      expectTargets();
      await fireEvent.press(screen.getByRole('button', { name: labels.closeLabel }), event());
      await fireEvent.press(screen.getByRole('button', { name: 'Progress, History' }), event());
      expect(screen.getAllByRole('menuitem')).toHaveLength(2);
      expectTargets();
      await rerender(<AppNavigation {...props} width={320} pathname="/" />);
      expect(screen.getAllByRole('link')).toHaveLength(2);
      expectTargets();
    } finally {
      Platform.OS = original;
    }
  },
);

it('waits for Modal presentation before moving accessibility focus', async () => {
  const focus = jest.spyOn(AccessibilityInfo, 'setAccessibilityFocus').mockReturnValue(undefined);
  const handle = jest.spyOn(nativeModules, 'findNodeHandle').mockReturnValue(42);
  try {
    await render(
      <SectionPicker
        closeLabel="Close"
        item={section}
        pathname="/progress/history"
        theme={theme}
        onNavigate={jest.fn()}
      />,
    );
    await fireEvent.press(screen.getByRole('button', { name: 'Progress, History' }), event());
    expect(focus).not.toHaveBeenCalled();
    const modal = screen.container.queryAll(
      (node) => typeof node.props['onShow'] === 'function',
    )[0];
    if (modal === undefined) throw new Error('Modal presentation callback is missing');
    await fireEvent(modal, 'show');
    expect(focus).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledWith(42);
    await fireEvent.press(screen.getByRole('button', { name: 'Close' }), event());
    expect(focus).toHaveBeenCalledTimes(2);
  } finally {
    focus.mockRestore();
    handle.mockRestore();
  }
});
it('hides navigation behind the profile menu from screen readers and restores it on close', async () => {
  await render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      theme={theme}
      onNavigate={jest.fn()}
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [{ id: 'settings', label: 'Settings', href: '/settings' }],
      }}
    />,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Account' }), event());
  const background = screen.getByTestId('primary-navigation', { includeHiddenElements: true });
  expect(background).toHaveProp('accessibilityElementsHidden', true);
  expect(background).toHaveProp('importantForAccessibility', 'no-hide-descendants');
  expect(screen.queryByRole('link', { name: 'Home' })).toBeNull();
  expect(screen.getByRole('menuitem', { name: 'Settings' })).toBeOnTheScreen();
  await fireEvent.press(screen.getByRole('button', { name: 'Close' }), event());
  expect(background).not.toHaveProp('accessibilityElementsHidden', true);
  expect(screen.getByRole('link', { name: 'Home' })).toBeOnTheScreen();
});
it('hides the section picker behind its modal while keeping the selected entry accessible', async () => {
  await render(
    <SectionPicker
      closeLabel="Close"
      item={section}
      pathname="/progress/history"
      theme={theme}
      onNavigate={jest.fn()}
    />,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Progress, History' }), event());
  expect(screen.queryByRole('button', { name: 'Progress, History' })).toBeNull();
  expect(
    screen.getByRole('button', { name: 'Progress, History', includeHiddenElements: true }).parent,
  ).toHaveProp('importantForAccessibility', 'no-hide-descendants');
  expect(screen.getByRole('menuitem', { name: 'History', selected: true })).toHaveProp(
    'tabIndex',
    0,
  );
  await fireEvent.press(screen.getByRole('button', { name: 'Close' }), event());
  expect(screen.getByRole('button', { name: 'Progress, History' })).toBeOnTheScreen();
});

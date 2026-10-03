import { fireEvent, render, screen, within } from '@testing-library/react-native';
import { Text } from 'react-native';
import {
  AppNavigation,
  SectionPicker,
  type NavigationIcon,
  type NavigationTheme,
} from './native.js';
import type { NavigationItem } from './index.js';

const icon: NavigationIcon = ({ active }) => <Text>{active ? 'filled' : 'outline'}</Text>;
const section: NavigationItem<NavigationIcon> = {
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
const theme: NavigationTheme = {
  background: '#fff',
  text: '#111',
  muted: '#595959',
  activeBackground: '#005fcc',
  activeText: '#fff',
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
        items={items}
        width={1024}
        pathname="/progress/history"
        theme={theme}
        onNavigate={jest.fn()}
      />
      <SectionPicker
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
  expect(active).toHaveStyle({ borderColor: theme.activeBackground });
  const picker = screen.getByRole('button', { name: 'Progress, History' });
  await fireEvent(picker, 'focus', event());
  expect(picker).toHaveStyle({ borderWidth: 3, borderColor: theme.focus });
  await fireEvent(picker, 'blur', event());
  expect(picker).toHaveStyle({ borderColor: theme.border });
  await fireEvent.press(picker, event());
  const entry = screen.getByRole('menuitem', { name: 'History' });
  await fireEvent(entry, 'focus', event());
  expect(entry).toHaveStyle({ borderWidth: 3, borderColor: theme.focus });
  await fireEvent(entry, 'blur', event());
  expect(entry).toHaveStyle({ borderColor: theme.background });
});

it('shows an active disclosure with its selected child directly inside its section', async () => {
  const navigate = jest.fn();
  await render(
    <AppNavigation
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
    backgroundColor: theme.activeBackground,
  });
  expect(screen.getByRole('link', { name: 'History' })).toHaveStyle({
    backgroundColor: theme.activeBackground,
  });
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
it('respects controlled collapse and route changes', async () => {
  const changed = jest.fn();
  const props = { items, width: 1024, theme, onNavigate: jest.fn(), onCollapsedChange: changed };
  const view = await render(<AppNavigation {...props} collapsed pathname="/" />);
  await fireEvent.press(screen.getByRole('button', { name: 'Progress' }), event());
  expect(changed).toHaveBeenCalledWith(false);
  expect(screen.queryByText('Progress')).toBeNull();
  await view.rerender(<AppNavigation {...props} collapsed={false} pathname="/progress/history" />);
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
        items={items}
        width={width}
        pathname="/progress"
        theme={theme}
        onNavigate={navigate}
      />,
    );
    await fireEvent.press(screen.getByRole('link', { name: 'Progress' }), event());
    expect(navigate).toHaveBeenLastCalledWith('/progress/history');
    await view.rerender(
      <AppNavigation
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
    <AppNavigation items={items} width={320} pathname="/" theme={theme} onNavigate={navigate} />,
  );
  const press = { nativeEvent: modifier, preventDefault: jest.fn() };
  await fireEvent.press(screen.getByRole('link', { name: 'Home' }), press);
  expect(navigate).not.toHaveBeenCalled();
  expect(press.preventDefault).not.toHaveBeenCalled();
});
it('uses roving tab stops for Arrow, Home and End', async () => {
  await render(
    <AppNavigation
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
    <SectionPicker item={section} pathname="/progress" theme={theme} onNavigate={navigate} />,
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

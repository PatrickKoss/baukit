import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { AppNavigation, SectionPicker, type NavigationIcon } from './web.js';
import type { NavigationItem, NavigationProfile } from './index.js';

const labels = {
  label: 'Primary',
  collapseLabel: 'Collapse navigation',
  expandLabel: 'Expand navigation',
  closeLabel: 'Close',
};

const icon: NavigationIcon = ({ active }) => <span>{active ? 'filled' : 'outline'}</span>;
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
const profile: NavigationProfile = { label: 'Account', initials: 'AB', href: '/profile' };
afterEach(cleanup);

describe('rail', () => {
  it('renders links and an active disclosure with children immediately below it', () => {
    const navigate = vi.fn();
    render(
      <AppNavigation
        {...labels}
        items={items}
        profile={profile}
        width={1024}
        pathname="/progress/history"
        onNavigate={navigate}
      />,
    );
    expect(screen.getByRole('link', { name: 'Home' }).getAttribute('href')).toBe('/');
    const parent = screen.getByRole('button', { name: 'Progress' });
    expect(parent.getAttribute('aria-expanded')).toBe('true');
    expect(parent.getAttribute('data-active')).toBe('ancestor');
    const activeChild = screen.getByRole('link', { name: 'History' });
    expect(activeChild.getAttribute('data-active')).toBe('page');
    expect(within(parent).getByText('filled')).toBeTruthy();
    expect(within(activeChild).getByText('filled')).toBeTruthy();
    expect(screen.getByRole('navigation').querySelectorAll('[data-active="page"]')).toHaveLength(1);
    expect(parent.nextElementSibling).toBe(screen.getByRole('group', { name: 'Progress' }));
    expect(parent.getAttribute('aria-controls')).toBe(parent.nextElementSibling?.id);
    expect(screen.getByRole('link', { name: 'History' }).getAttribute('aria-current')).toBe('page');
    fireEvent.click(parent);
    expect(parent.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('link', { name: 'History' })).toBeNull();
    expect(navigate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('link', { name: 'Home' }));
    expect(navigate).toHaveBeenCalledWith('/', expect.objectContaining({ defaultPrevented: true }));
  });
  it('fills the active parent in a collapsed rail and its child after expanding', () => {
    render(
      <AppNavigation
        {...labels}
        items={items}
        width={1024}
        pathname="/progress/history"
        defaultCollapsed
      />,
    );
    const parent = screen.getByRole('button', { name: 'Progress' });
    expect(parent.getAttribute('data-active')).toBe('page');
    expect(screen.queryByRole('link', { name: 'History' })).toBeNull();
    fireEvent.click(parent);
    expect(parent.getAttribute('data-active')).toBe('ancestor');
    expect(screen.getByRole('link', { name: 'History' }).getAttribute('data-active')).toBe('page');
  });
  it('fills an active main item without children', () => {
    render(<AppNavigation {...labels} items={items} width={1024} pathname="/" />);
    expect(screen.getByRole('link', { name: 'Home' }).getAttribute('data-active')).toBe('page');
    expect(screen.getByRole('button', { name: 'Progress' }).hasAttribute('data-active')).toBe(
      false,
    );
  });
  it('expands a collapsed rail and opens the clicked group', () => {
    const changed = vi.fn();
    render(
      <AppNavigation
        {...labels}
        items={items}
        width={1024}
        pathname="/"
        defaultCollapsed
        onCollapsedChange={changed}
      />,
    );
    expect(screen.getByRole('button', { name: 'Progress' }).title).toBe('Progress');
    fireEvent.click(screen.getByRole('button', { name: 'Progress' }));
    expect(changed).toHaveBeenCalledWith(false);
    expect(screen.getByRole('navigation').getAttribute('data-collapsed')).toBe('false');
    expect(screen.getByRole('button', { name: 'Progress' }).getAttribute('aria-expanded')).toBe(
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Collapse navigation' }));
    expect(screen.getByRole('navigation').getAttribute('data-collapsed')).toBe('true');
    expect(screen.queryByRole('link', { name: 'History' })).toBeNull();
  });
  it('connects disclosures when product ids contain spaces', () => {
    render(
      <AppNavigation
        {...labels}
        items={[{ ...section, id: 'training progress' }]}
        width={1024}
        pathname="/progress"
      />,
    );
    const parent = screen.getByRole('button', { name: 'Progress' });
    const controls = parent.getAttribute('aria-controls');
    expect(controls).not.toMatch(/\s/);
    expect(document.getElementById(controls ?? '')).toBe(
      screen.getByRole('group', { name: 'Progress' }),
    );
  });
  it('respects controlled collapse and opens sections when the route changes', () => {
    const changed = vi.fn();
    const props = { items, width: 1024, onCollapsedChange: changed };
    const { rerender } = render(<AppNavigation {...labels} {...props} pathname="/" collapsed />);
    fireEvent.click(screen.getByRole('button', { name: 'Progress' }));
    expect(changed).toHaveBeenCalledWith(false);
    expect(screen.getByRole('navigation').getAttribute('data-collapsed')).toBe('true');
    rerender(
      <AppNavigation {...labels} {...props} pathname="/progress/history" collapsed={false} />,
    );
    expect(screen.getByRole('button', { name: 'Progress' }).getAttribute('aria-expanded')).toBe(
      'true',
    );
  });
  it('moves focus with arrows, Home and End through visible children and profile', () => {
    render(
      <AppNavigation
        {...labels}
        items={items}
        profile={profile}
        width={1024}
        pathname="/progress"
      />,
    );
    const home = screen.getByRole('link', { name: 'Home' });
    home.focus();
    fireEvent.keyDown(home, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Progress' }));
    fireEvent.keyDown(document.activeElement ?? home, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Overview' }));
    fireEvent.keyDown(document.activeElement ?? home, { key: 'End' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Account' }));
    fireEvent.keyDown(document.activeElement ?? home, { key: 'Home' });
    expect(document.activeElement).toBe(home);
  });
});
it.each([
  { metaKey: true },
  { ctrlKey: true },
  { shiftKey: true },
  { altKey: true },
  { button: 1 },
])('leaves modified clicks to the browser: %j', (modifier) => {
  const navigate = vi.fn();
  render(
    <AppNavigation {...labels} items={items} width={320} pathname="/" onNavigate={navigate} />,
  );
  expect(fireEvent.click(screen.getByRole('link', { name: 'Home' }), modifier)).toBe(true);
  expect(navigate).not.toHaveBeenCalled();
});
it('renders a real anchor through the optional link renderer', () => {
  render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      renderLink={(props) => <a {...props} data-router="custom" />}
    />,
  );
  expect(screen.getByRole('link', { name: 'Home' }).getAttribute('data-router')).toBe('custom');
});
it('rotates active compact tabs and sends inactive tabs to their root', () => {
  const navigate = vi.fn();
  const { rerender } = render(
    <AppNavigation
      {...labels}
      items={items}
      width={1023}
      pathname="/progress"
      onNavigate={navigate}
    />,
  );
  fireEvent.click(screen.getByRole('link', { name: 'Progress' }));
  expect(screen.getByRole('link', { name: 'Progress' }).getAttribute('data-active')).toBe('page');
  expect(navigate).toHaveBeenLastCalledWith('/progress/history', expect.anything());
  rerender(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/progress/history"
      onNavigate={navigate}
    />,
  );
  fireEvent.click(screen.getByRole('link', { name: 'Progress' }));
  expect(navigate).toHaveBeenLastCalledWith('/progress', expect.anything());
  fireEvent.click(screen.getByRole('link', { name: 'Home' }));
  expect(navigate).toHaveBeenLastCalledWith('/', expect.anything());
});
it.each([320, 1024])('keeps profile last at width %i', (width) => {
  render(
    <AppNavigation {...labels} items={items} profile={profile} width={width} pathname="/profile" />,
  );
  const links = within(screen.getByRole('navigation')).getAllByRole('link');
  expect(links.at(-1)).toBe(screen.getByRole('link', { name: 'Account' }));
  expect(links.at(-1)?.getAttribute('data-active')).toBe('page');
  expect(links.at(-1)?.getAttribute('aria-current')).toBe('page');
});
it('handles profile menu focus, disabled items, actions, Escape and outside clicks', () => {
  const action = vi.fn();
  render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [
          { id: 'disabled', label: 'Unavailable', href: '/disabled', disabled: true },
          { id: 'settings', label: 'Settings', href: '/settings' },
          { id: 'signout', label: 'Sign out', onSelect: action },
        ],
      }}
    />,
  );
  const trigger = screen.getByRole('button', { name: 'Account' });
  fireEvent.keyDown(trigger, { key: 'ArrowDown' });
  expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Settings' }));
  fireEvent.keyDown(document.activeElement ?? trigger, { key: 'End' });
  expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Sign out' }));
  fireEvent.keyDown(document.activeElement ?? trigger, { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(trigger);
  fireEvent.click(trigger);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Sign out' }));
  expect(action).toHaveBeenCalledOnce();
  expect(screen.queryByRole('menu')).toBeNull();
  fireEvent.click(trigger);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole('menu')).toBeNull();
  fireEvent.click(trigger);
  fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Settings' }), { key: 'Tab' });
  expect(screen.queryByRole('menu')).toBeNull();
});
it('lets a picker select a section page directly', () => {
  const navigate = vi.fn();
  render(<SectionPicker item={section} pathname="/progress" onNavigate={navigate} />);
  fireEvent.click(screen.getByRole('button', { name: 'Progress, Overview' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'History' }));
  expect(navigate).toHaveBeenCalledWith('/progress/history', expect.anything());
  expect(screen.queryByRole('menu')).toBeNull();
});
it('keeps an all-disabled profile menu dismissible with the keyboard', () => {
  render(
    <AppNavigation
      {...labels}
      items={items}
      width={320}
      pathname="/"
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [{ id: 'unavailable', label: 'Unavailable', href: '/settings', disabled: true }],
      }}
    />,
  );
  const trigger = screen.getByRole('button', { name: 'Account' });
  fireEvent.keyDown(trigger, { key: 'ArrowDown' });
  const menu = screen.getByRole('menu');
  expect(document.activeElement).toBe(menu);
  fireEvent.keyDown(menu, { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it('marks only the most-specific profile route current with query and hash hrefs', () => {
  render(
    <AppNavigation
      {...labels}
      items={items}
      pathname="/settings/profile?tab=data#top"
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [
          { id: 'settings', label: 'App settings', href: '/settings?tab=app' },
          { id: 'profile-data', label: 'Profile and data', href: '/settings/profile?tab=data#top' },
          { id: 'account-alias', label: 'Alias', href: '/account', matches: () => false },
        ],
      }}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Account' }));
  const menu = screen.getByRole('menu');
  expect(menu.querySelectorAll('[data-active="page"]')).toHaveLength(1);
  expect(menu.querySelectorAll('[aria-current="page"]')).toHaveLength(1);
  expect(
    screen.getByRole('menuitem', { name: 'Profile and data' }).getAttribute('aria-current'),
  ).toBe('page');
  expect(screen.getByRole('menuitem', { name: 'App settings' }).hasAttribute('data-active')).toBe(
    false,
  );
});
it('uses a profile entry matcher instead of its href', () => {
  render(
    <AppNavigation
      {...labels}
      items={items}
      pathname="/alias?tab=profile"
      profile={{
        label: 'Account',
        initials: 'AB',
        menu: [
          {
            id: 'profile-data',
            label: 'Profile and data',
            href: '/settings/profile',
            matches: (path) => path === '/alias?tab=profile',
          },
        ],
      }}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Account' }));
  expect(
    screen.getByRole('menuitem', { name: 'Profile and data' }).getAttribute('aria-current'),
  ).toBe('page');
});
it('uses product labels for the navigation and collapse controls', () => {
  render(
    <AppNavigation
      items={items}
      pathname="/"
      width={1024}
      label="Hauptnavigation"
      collapseLabel="Einklappen"
      expandLabel="Ausklappen"
    />,
  );
  expect(screen.getByRole('navigation', { name: 'Hauptnavigation' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Einklappen' }));
  expect(screen.getByRole('button', { name: 'Ausklappen' })).toBeTruthy();
});

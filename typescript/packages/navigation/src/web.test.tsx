import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { AppNavigation, SectionPicker, type NavigationIcon } from './web.js';
import type { NavigationItem, NavigationProfile } from './index.js';

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
    expect(parent.getAttribute('data-active')).toBe('true');
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
  it('expands a collapsed rail and opens the clicked group', () => {
    const changed = vi.fn();
    render(
      <AppNavigation
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
    const { rerender } = render(<AppNavigation {...props} pathname="/" collapsed />);
    fireEvent.click(screen.getByRole('button', { name: 'Progress' }));
    expect(changed).toHaveBeenCalledWith(false);
    expect(screen.getByRole('navigation').getAttribute('data-collapsed')).toBe('true');
    rerender(<AppNavigation {...props} pathname="/progress/history" collapsed={false} />);
    expect(screen.getByRole('button', { name: 'Progress' }).getAttribute('aria-expanded')).toBe(
      'true',
    );
  });
  it('moves focus with arrows, Home and End through visible children and profile', () => {
    render(<AppNavigation items={items} profile={profile} width={1024} pathname="/progress" />);
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
  render(<AppNavigation items={items} width={320} pathname="/" onNavigate={navigate} />);
  expect(fireEvent.click(screen.getByRole('link', { name: 'Home' }), modifier)).toBe(true);
  expect(navigate).not.toHaveBeenCalled();
});
it('renders a real anchor through the optional link renderer', () => {
  render(
    <AppNavigation
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
    <AppNavigation items={items} width={1023} pathname="/progress" onNavigate={navigate} />,
  );
  fireEvent.click(screen.getByRole('link', { name: 'Progress' }));
  expect(navigate).toHaveBeenLastCalledWith('/progress/history', expect.anything());
  rerender(
    <AppNavigation items={items} width={320} pathname="/progress/history" onNavigate={navigate} />,
  );
  fireEvent.click(screen.getByRole('link', { name: 'Progress' }));
  expect(navigate).toHaveBeenLastCalledWith('/progress', expect.anything());
  fireEvent.click(screen.getByRole('link', { name: 'Home' }));
  expect(navigate).toHaveBeenLastCalledWith('/', expect.anything());
});
it.each([320, 1024])('keeps profile last at width %i', (width) => {
  render(<AppNavigation items={items} profile={profile} width={width} pathname="/profile" />);
  const links = within(screen.getByRole('navigation')).getAllByRole('link');
  expect(links.at(-1)).toBe(screen.getByRole('link', { name: 'Account' }));
  expect(links.at(-1)?.getAttribute('aria-current')).toBe('page');
});
it('handles profile menu focus, disabled items, actions, Escape and outside clicks', () => {
  const action = vi.fn();
  render(
    <AppNavigation
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

import { describe, expect, it } from 'vitest';
import { blendColors, contrastRatio, exampleTokens } from '@baukit/ui-tokens';
import {
  getNavigationLayout,
  navigationReducer,
  navigationMatches,
  resolveActiveMenuEntry,
  nextSectionHref,
  resolveActiveNavigation,
  validateNavigation,
  type NavigationItem,
} from './index.js';

const items: readonly NavigationItem<string>[] = [
  { id: 'home', label: 'Home', href: '/', icon: 'home' },
  {
    id: 'progress',
    label: 'Progress',
    href: '/progress',
    icon: 'chart',
    children: [
      { id: 'overview', label: 'Overview', href: '/progress' },
      { id: 'history', label: 'History', href: '/progress/history' },
      {
        id: 'report',
        label: 'Report',
        href: '/report',
        matches: (path) => path.startsWith('/reports/'),
      },
    ],
  },
];
const section = items[1];
if (section === undefined) throw new Error('Missing fixture section');

describe('active navigation', () => {
  it('selects the most specific child and its parent', () => {
    expect(resolveActiveNavigation(items, '/progress/history/42')).toEqual({
      item: section,
      subItem: section.children?.[1],
    });
    expect(resolveActiveNavigation(items, '/reports/2026')).toEqual({
      item: section,
      subItem: section.children?.[2],
    });
    expect(resolveActiveNavigation(items, '/')).toEqual({ item: items[0], subItem: null });
    expect(resolveActiveNavigation(items, '/progressive')).toEqual({ item: null, subItem: null });
    expect(resolveActiveNavigation([], '/')).toEqual({ item: null, subItem: null });
  });
  it('uses caller matches instead of the href fallback', () => {
    expect(
      resolveActiveNavigation(
        [{ ...section, children: [], matches: (path) => path === '/other' }],
        '/progress',
      ).item,
    ).toBeNull();
    expect(
      resolveActiveNavigation(
        [{ ...section, children: [], matches: (path) => path === '/other' }],
        '/other',
      ).item?.id,
    ).toBe('progress');
  });
});
describe('section rotation', () => {
  it('rotates, wraps and starts at the first route when none matches', () => {
    expect(nextSectionHref(section, '/progress')).toBe('/progress/history');
    expect(nextSectionHref(section, '/progress/history/42')).toBe('/report');
    expect(nextSectionHref(section, '/reports/2026')).toBe('/progress');
    expect(nextSectionHref(section, '/missing')).toBe('/progress');
    expect(nextSectionHref({ ...section, children: [] }, '/missing')).toBe('/progress');
  });
});
describe('disclosure state', () => {
  it('opens the active section, toggles and preserves groups through collapse', () => {
    const initial = { collapsed: false, openIds: [] };
    const open = navigationReducer(initial, { type: 'enter-section', id: 'progress' });
    expect(open.openIds).toEqual(['progress']);
    expect(navigationReducer(open, { type: 'enter-section', id: 'progress' })).toBe(open);
    expect(navigationReducer(open, { type: 'toggle-group', id: 'progress' }).openIds).toEqual([]);
    const collapsed = navigationReducer(open, { type: 'set-collapsed', collapsed: true });
    expect(collapsed).toEqual({ collapsed: true, openIds: ['progress'] });
    expect(navigationReducer(collapsed, { type: 'toggle-group', id: 'progress' })).toEqual(open);
    expect(
      navigationReducer({ collapsed: true, openIds: [] }, { type: 'toggle-group', id: 'progress' }),
    ).toEqual(open);
    expect(navigationReducer(initial, { type: 'enter-section', id: null })).toBe(initial);
  });
});
describe('validation', () => {
  it('accepts a profile link and a profile menu', () => {
    expect(() => {
      validateNavigation(items, { label: 'Account', initials: 'AB', href: '/profile' });
    }).not.toThrow();
    expect(() => {
      validateNavigation(items, {
        label: 'Account',
        initials: 'AB',
        menu: [{ id: 'signout', label: 'Sign out', onSelect: () => undefined }],
      });
    }).not.toThrow();
  });
  it('rejects duplicate ids across parents, children and profile entries', () => {
    expect(() => {
      validateNavigation([...items, { ...section, id: 'history' }]);
    }).toThrow(/id/);
    expect(() => {
      validateNavigation(items, {
        label: 'Account',
        initials: 'A',
        menu: [{ id: 'home', label: 'Settings', href: '/settings' }],
      });
    }).toThrow(/id/);
  });
  it('rejects missing hrefs, labels, ids, initials and empty menus', () => {
    for (const invalid of [
      { ...section, href: '' },
      { ...section, label: '' },
      { ...section, id: '' },
    ])
      expect(() => {
        validateNavigation([invalid]);
      }).toThrow();
    expect(() => {
      validateNavigation(items, { label: 'Account', initials: '', href: '/profile' });
    }).toThrow(/initials/);
    expect(() => {
      validateNavigation(items, { label: 'Account', initials: 'A', href: '' });
    }).toThrow(/href/);
    expect(() => {
      validateNavigation(items, { label: 'Account', initials: 'A', menu: [] });
    }).toThrow(/empty/);
    expect(() => {
      validateNavigation(items, {
        label: 'Account',
        initials: 'A',
        menu: [{ id: 'settings', label: 'Settings', href: '' }],
      });
    }).toThrow(/empty/);
  });
  it('rejects more than five main items', () => {
    expect(() => {
      validateNavigation(
        Array.from({ length: 6 }, (_, i) => ({
          id: String(i),
          href: `/${String(i)}`,
          label: 'Page',
          icon: 'page',
        })),
      );
    }).toThrow(/five/);
  });
});
it('uses the token layout helper at the rail boundary', () => {
  expect([320, 600, 1023, 1024].map(getNavigationLayout)).toEqual(['bar', 'bar', 'bar', 'rail']);
});
it.each(['light', 'dark'] as const)(
  'keeps muted active text readable in the %s theme',
  (scheme) => {
    const accent = exampleTokens.color.background.accent[scheme];
    const background = exampleTokens.color.background.primary[scheme];
    expect(contrastRatio(accent, blendColors(accent, background, 0.14))).toBeGreaterThanOrEqual(
      4.5,
    );
    expect(contrastRatio(accent, background)).toBeGreaterThanOrEqual(4.5);
  },
);

it('matches paths without query or hash and preserves explicit matchers', () => {
  expect(
    navigationMatches(
      { id: 'route', label: 'Route', href: '/settings?tab=app#top' },
      '/settings#other',
    ),
  ).toBe(true);
  expect(
    navigationMatches(
      { id: 'route', label: 'Route', href: '/settings?tab=app' },
      '/settings/profile?tab=data',
    ),
  ).toBe(true);
  expect(navigationMatches({ id: 'route', label: 'Route', href: '/?tab=home' }, '/settings')).toBe(
    false,
  );
  expect(
    navigationMatches(
      {
        id: 'route',
        label: 'Route',
        href: '/settings',
        matches: (path) => path === '/settings?tab=app',
      },
      '/settings?tab=app',
    ),
  ).toBe(true);
  expect(
    navigationMatches(
      {
        id: 'route',
        label: 'Route',
        href: '/settings',
        matches: (path) => path === '/settings?tab=app',
      },
      '/settings?tab=data',
    ),
  ).toBe(false);
});
it('selects one most-specific menu route using its path length', () => {
  const settings = { id: 'settings', label: 'App settings', href: '/settings?long=query-string' };
  const profile = { id: 'profile', label: 'Profile and data', href: '/settings/profile' };
  const action = { id: 'signout', label: 'Sign out', onSelect: () => undefined };
  const entries = [settings, profile, action];
  expect(resolveActiveMenuEntry(entries, '/settings/profile/data')).toBe(profile);
  expect(resolveActiveMenuEntry(entries, '/settings')).toBe(settings);
  expect(resolveActiveMenuEntry(entries, '/missing')).toBeNull();
  const overridden = { ...profile, matches: (path: string) => path === '/account' };
  expect(resolveActiveMenuEntry([settings, overridden], '/settings/profile')).toBe(settings);
  expect(resolveActiveMenuEntry([settings, overridden], '/account')).toBe(overridden);
});

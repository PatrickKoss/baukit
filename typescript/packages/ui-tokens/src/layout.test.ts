import { describe, expect, it } from 'vitest';

import {
  getLayoutMode,
  getScreenMaxWidth,
  getTabContentInset,
  type LayoutBreakpoints,
} from './layout.js';

const BREAKPOINTS: LayoutBreakpoints = { medium: 768, expanded: 1024 };

const MAX_WIDTHS = { form: 720, reading: 720, dashboard: 1200 } as const;

const WIDTH_OPTIONS = {
  maxWidths: MAX_WIDTHS,
  narrowFallback: 'reading',
  expandedOnly: ['dashboard'],
} as const;

describe('getLayoutMode', () => {
  it.each([
    [0, 'compact'],
    [767, 'compact'],
    [768, 'medium'],
    [1023, 'medium'],
    [1024, 'expanded'],
    [1920, 'expanded'],
  ])('classifies width %i as %s', (width, expected) => {
    expect(getLayoutMode(width, BREAKPOINTS)).toBe(expected);
  });

  it('follows the breakpoints it is given, not a built-in set', () => {
    const wide: LayoutBreakpoints = { medium: 1000, expanded: 1600 };

    expect(getLayoutMode(900, wide)).toBe('compact');
    expect(getLayoutMode(900, BREAKPOINTS)).toBe('medium');
  });
});

describe('getScreenMaxWidth', () => {
  it('returns the named width when the mode allows it', () => {
    expect(getScreenMaxWidth('dashboard', 'expanded', WIDTH_OPTIONS)).toBe(1200);
    expect(getScreenMaxWidth('form', 'compact', WIDTH_OPTIONS)).toBe(720);
  });

  it.each(['compact', 'medium'] as const)('narrows an expanded-only screen in %s', (mode) => {
    expect(getScreenMaxWidth('dashboard', mode, WIDTH_OPTIONS)).toBe(MAX_WIDTHS.reading);
  });

  it('leaves screens outside the expanded-only list alone', () => {
    expect(getScreenMaxWidth('form', 'medium', WIDTH_OPTIONS)).toBe(720);
  });

  it('rejects a screen name the product did not define', () => {
    // Products that build the option set at runtime lose the compile-time check.
    const unknown = 'missing' as keyof typeof MAX_WIDTHS;

    expect(() => getScreenMaxWidth(unknown, 'expanded', WIDTH_OPTIONS)).toThrow(
      'unknown screen max width: missing',
    );
  });
});

describe('getTabContentInset', () => {
  it('adds the tab bar and safe area below the expanded mode', () => {
    expect(getTabContentInset('compact', 34, 56)).toBe(90);
    expect(getTabContentInset('medium', 0, 56)).toBe(56);
  });

  it('needs no inset once the tab bar becomes a rail', () => {
    expect(getTabContentInset('expanded', 34, 56)).toBe(0);
  });

  it('ignores a negative safe area', () => {
    expect(getTabContentInset('compact', -10, 56)).toBe(56);
  });
});

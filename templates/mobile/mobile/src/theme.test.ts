import { checkContrast, validateTokens } from '@baukit/ui-tokens';

import {
  applyWebFocusColor,
  createNavigationTheme,
  darkTheme,
  lightTheme,
  resolveThemeScheme,
  themeTokenSource,
} from './theme';

describe('theme resolution', () => {
  it('follows changes to the OS scheme in system mode', () => {
    expect(resolveThemeScheme('system', 'dark')).toBe('dark');
    expect(resolveThemeScheme('system', 'light')).toBe('light');
  });

  it('keeps explicit light and dark modes when the OS scheme changes', () => {
    expect(resolveThemeScheme('light', 'dark')).toBe('light');
    expect(resolveThemeScheme('dark', 'light')).toBe('dark');
  });

  it.each([
    ['light', lightTheme],
    ['dark', darkTheme],
  ] as const)(
    'provides a validated %s semantic token tree',
    (scheme, theme) => {
      expect(validateTokens(themeTokenSource)).toEqual([]);
      expect(checkContrast(themeTokenSource)).toEqual([]);
      expect(theme.scheme).toBe(scheme);
      expect(
        Object.values(theme.color).every((color) => color.length > 0),
      ).toBe(true);
    },
  );

  it('maps semantic colors into the navigation theme', () => {
    const navigationTheme = createNavigationTheme(darkTheme);

    expect(navigationTheme.dark).toBe(true);
    expect(navigationTheme.colors).toMatchObject({
      background: darkTheme.color.background,
      card: darkTheme.color.surface,
      primary: darkTheme.color.accent,
      text: darkTheme.color.text,
    });
  });

  it('applies the focus color variable on web only', () => {
    const setProperty = jest.fn();
    const root = { style: { setProperty } };

    applyWebFocusColor(lightTheme.color.focus, 'web', root);
    expect(setProperty).toHaveBeenCalledWith(
      '--app-focus-color',
      lightTheme.color.focus,
    );

    applyWebFocusColor(darkTheme.color.focus, 'ios', root);
    expect(setProperty).toHaveBeenCalledTimes(1);
  });
});

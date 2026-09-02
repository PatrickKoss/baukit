import {
  createContext,
  createElement,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useMemo,
} from 'react';
import {
  DarkTheme,
  DefaultTheme,
  ThemeProvider as NavigationThemeProvider,
} from 'expo-router';
import type { Theme as NavigationTheme } from 'expo-router';
import { Platform, StatusBar, useColorScheme } from 'react-native';
import { parseTokens, type DesignTokens } from '@baukit/ui-tokens';

import type { ThemePreference } from './app-preferences';
import { tokens } from './tokens';

export type ResolvedThemeScheme = Exclude<ThemePreference, 'system'>;

export interface AppTheme {
  readonly scheme: ResolvedThemeScheme;
  readonly color: {
    readonly background: string;
    readonly surface: string;
    readonly text: string;
    readonly muted: string;
    readonly accent: string;
    readonly onAccent: string;
    readonly border: string;
    readonly error: string;
    readonly focus: string;
  };
  readonly space: {
    readonly small: number;
    readonly medium: number;
    readonly large: number;
  };
  readonly radius: {
    readonly button: number;
    readonly card: number;
  };
}

export interface ThemeContextValue {
  readonly mode: ThemePreference;
  readonly resolvedScheme: ResolvedThemeScheme;
  readonly theme: AppTheme;
  readonly navigationTheme: NavigationTheme;
  readonly setMode: (mode: ThemePreference) => Promise<void>;
}

export interface AppThemeProviderProps extends PropsWithChildren {
  readonly mode: ThemePreference;
  readonly persistMode: (mode: ThemePreference) => Promise<void>;
}

export const themeTokenSource = parseTokens({
  color: {
    background: {
      primary: {
        light: tokens.color.background.primary.light,
        dark: tokens.color.background.primary.dark,
      },
      surface: { light: '#f4f6f8', dark: '#1c1f23' },
      accent: {
        light: tokens.color.background.accent.light,
        dark: tokens.color.background.accent.dark,
      },
    },
    text: {
      primary: {
        light: tokens.color.text.primary.light,
        dark: tokens.color.text.primary.dark,
      },
      muted: { light: '#59636e', dark: '#b3bbc4' },
      onAccent: {
        light: tokens.color.text.onAccent.light,
        dark: tokens.color.text.onAccent.dark,
      },
    },
    border: {
      default: { light: '#d8dde3', dark: '#46505a' },
    },
    feedback: {
      error: { light: '#b42318', dark: '#ff8a80' },
    },
    focus: {
      ring: { light: '#111111', dark: '#ffffff' },
    },
  },
  typography: tokens.typography,
  space: tokens.space,
  radius: tokens.radius,
  motion: tokens.motion,
  elevation: tokens.elevation,
  contrastPairs: [
    {
      foreground: 'color.text.primary',
      background: 'color.background.primary',
    },
    {
      foreground: 'color.text.onAccent',
      background: 'color.background.accent',
    },
  ],
} satisfies DesignTokens);

type ThemeColors = typeof themeTokenSource.color;

function themeColor(
  colors: ThemeColors,
  path: readonly string[],
  scheme: ResolvedThemeScheme,
): string {
  let value: unknown = colors;
  for (const segment of path) {
    value = (value as Record<string, unknown>)[segment];
  }
  return (value as Record<ResolvedThemeScheme, string>)[scheme];
}

function createTheme(scheme: ResolvedThemeScheme): AppTheme {
  const colors = themeTokenSource.color;
  return {
    scheme,
    color: {
      background: themeColor(colors, ['background', 'primary'], scheme),
      surface: themeColor(colors, ['background', 'surface'], scheme),
      text: themeColor(colors, ['text', 'primary'], scheme),
      muted: themeColor(colors, ['text', 'muted'], scheme),
      accent: themeColor(colors, ['background', 'accent'], scheme),
      onAccent: themeColor(colors, ['text', 'onAccent'], scheme),
      border: themeColor(colors, ['border', 'default'], scheme),
      error: themeColor(colors, ['feedback', 'error'], scheme),
      focus: themeColor(colors, ['focus', 'ring'], scheme),
    },
    space: {
      small: tokens.space.small,
      medium: tokens.space.medium,
      large: 24,
    },
    radius: {
      button: tokens.radius.small,
      card: 12,
    },
  };
}

export const lightTheme = createTheme('light');
export const darkTheme = createTheme('dark');

const themes: Readonly<Record<ResolvedThemeScheme, AppTheme>> = {
  light: lightTheme,
  dark: darkTheme,
};

export function resolveThemeScheme(
  mode: ThemePreference,
  systemScheme: ReturnType<typeof useColorScheme>,
): ResolvedThemeScheme {
  if (mode !== 'system') {
    return mode;
  }
  return systemScheme === 'dark' ? 'dark' : 'light';
}

export function createNavigationTheme(theme: AppTheme): NavigationTheme {
  const base = theme.scheme === 'dark' ? DarkTheme : DefaultTheme;
  return {
    ...base,
    colors: {
      primary: theme.color.accent,
      background: theme.color.background,
      card: theme.color.surface,
      text: theme.color.text,
      border: theme.color.border,
      notification: theme.color.error,
    },
  };
}

interface FocusStyleTarget {
  readonly style: {
    setProperty(name: string, value: string): void;
  };
}

export function applyWebFocusColor(
  focusColor: string,
  platform: string = Platform.OS,
  root: FocusStyleTarget | undefined = typeof document === 'undefined'
    ? undefined
    : document.documentElement,
): void {
  if (platform === 'web') {
    root?.style.setProperty('--app-focus-color', focusColor);
  }
}

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

export function AppThemeProvider({
  children,
  mode,
  persistMode,
}: AppThemeProviderProps) {
  const systemScheme = useColorScheme();
  const resolvedScheme = resolveThemeScheme(mode, systemScheme);
  const theme = themes[resolvedScheme];
  const navigationTheme = useMemo(() => createNavigationTheme(theme), [theme]);

  useEffect(() => {
    StatusBar.setBarStyle(
      resolvedScheme === 'dark' ? 'light-content' : 'dark-content',
      true,
    );
  }, [resolvedScheme]);

  useEffect(() => {
    applyWebFocusColor(theme.color.focus);
  }, [theme.color.focus]);

  const setMode = useCallback(
    (nextMode: ThemePreference) => persistMode(nextMode),
    [persistMode],
  );
  const value = useMemo<ThemeContextValue>(
    () => ({ mode, navigationTheme, resolvedScheme, setMode, theme }),
    [mode, navigationTheme, resolvedScheme, setMode, theme],
  );

  return createElement(NavigationThemeProvider, {
    value: navigationTheme,
    children: createElement(ThemeContext.Provider, { value }, children),
  });
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (value === undefined) {
    throw new Error('useTheme must be used within AppThemeProvider.');
  }
  return value;
}

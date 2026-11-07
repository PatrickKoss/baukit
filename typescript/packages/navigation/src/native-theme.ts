import type { ReactNode } from 'react';
import { Platform, type TextStyle, type ViewStyle } from 'react-native';
import {
  getNavigationBarLineHeight,
  NAVIGATION_BAR_DIMENSIONS,
  type NavigationBarMetrics,
} from './index.js';

export interface NavigationIconState {
  readonly active: boolean;
  readonly size: number;
}
export type NavigationIcon = (state: NavigationIconState) => ReactNode;
export interface NavigationTypography {
  readonly fontFamily?: string;
  readonly activeFontFamily?: string;
  readonly fontSize?: number;
  readonly barFontSize?: number;
  readonly subtitleFontSize?: number;
  readonly fontWeight?: TextStyle['fontWeight'];
  readonly activeFontWeight?: TextStyle['fontWeight'];
  readonly lineHeight?: number;
  readonly letterSpacing?: number;
}

export interface NavigationTheme {
  readonly background: string;
  readonly text: string;
  readonly muted: string;
  readonly danger: string;
  readonly activeBackground: string;
  readonly activeText: string;
  readonly ancestorText: string;
  readonly border: string;
  readonly focus: string;
  readonly radius: number;
  readonly spacing: number;
  readonly typography?: NavigationTypography;
}

export interface NavigationInsets {
  readonly top?: number;
  readonly bottom?: number;
  readonly left?: number;
  readonly right?: number;
}
export type Navigate = (href: string, options?: { readonly replace: boolean }) => void;

const DEFAULT_FONT_SIZE = 14;
const DEFAULT_SUBTITLE_FONT_SIZE = 12;
export const MENU_WIDTH = 280;
export const MENU_EDGE = 8;

export function barMetrics(theme?: NavigationTheme): NavigationBarMetrics {
  return {
    ...(theme === undefined ? {} : { spacing: theme.spacing }),
    ...(theme?.typography?.barFontSize === undefined
      ? {}
      : { fontSize: theme.typography.barFontSize }),
    ...(theme?.typography?.lineHeight === undefined
      ? {}
      : { lineHeight: theme.typography.lineHeight }),
  };
}

function fontFamily(typography: NavigationTypography, selected: boolean): string | undefined {
  return selected ? (typography.activeFontFamily ?? typography.fontFamily) : typography.fontFamily;
}

function fontSize(typography: NavigationTypography, bar: boolean): number {
  return bar
    ? (typography.barFontSize ?? NAVIGATION_BAR_DIMENSIONS.labelFontSize)
    : (typography.fontSize ?? DEFAULT_FONT_SIZE);
}

function fontWeight(typography: NavigationTypography, selected: boolean): TextStyle['fontWeight'] {
  return selected ? (typography.activeFontWeight ?? '700') : (typography.fontWeight ?? '400');
}

export function textStyle(theme: NavigationTheme, selected = false, bar = false): TextStyle {
  const typography = theme.typography ?? {};
  return {
    fontFamily: fontFamily(typography, selected),
    fontSize: fontSize(typography, bar),
    fontWeight: fontWeight(typography, selected),
    lineHeight: bar ? getNavigationBarLineHeight(barMetrics(theme)) : typography.lineHeight,
    letterSpacing: typography.letterSpacing,
  };
}

export function subtitleTextStyle(theme: NavigationTheme): TextStyle {
  return {
    ...textStyle(theme),
    color: theme.muted,
    fontSize: theme.typography?.subtitleFontSize ?? DEFAULT_SUBTITLE_FONT_SIZE,
  };
}

export function menuModalPadding(insets: NavigationInsets = {}): ViewStyle {
  const native = Platform.OS !== 'web';
  const edge = (inset: number | undefined): number => MENU_EDGE + (native ? (inset ?? 0) : 0);
  return {
    paddingTop: edge(insets.top),
    paddingBottom: edge(insets.bottom),
    paddingLeft: edge(insets.left),
    paddingRight: edge(insets.right),
  };
}

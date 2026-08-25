import type { DesignTokens, ThemeColor } from './schema.js';

export type ThemeName = 'light' | 'dark';

export interface ContrastViolation {
  readonly foreground: string;
  readonly background: string;
  readonly theme: ThemeName;
  readonly ratio: number;
  readonly requiredRatio: 3 | 4.5;
}

function expandHex(color: string): string {
  const match = /^#([\dA-Fa-f]{3}|[\dA-Fa-f]{6})$/u.exec(color);
  if (match?.[1] === undefined) {
    throw new TypeError(`Unsupported color ${color}; expected #RGB or #RRGGBB.`);
  }
  const digits = match[1];
  return digits.length === 3
    ? `${digits.charAt(0)}${digits.charAt(0)}${digits.charAt(1)}${digits.charAt(1)}${digits.charAt(2)}${digits.charAt(2)}`
    : digits;
}

function channel(hex: string, offset: number): number {
  const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.x relative luminance for an opaque hexadecimal sRGB color. */
export function relativeLuminance(color: string): number {
  const hex = expandHex(color);
  return 0.2126 * channel(hex, 0) + 0.7152 * channel(hex, 2) + 0.0722 * channel(hex, 4);
}

/** WCAG contrast ratio, always between 1 and 21 inclusive. */
export function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

function colorAt(tokens: DesignTokens, path: string): ThemeColor {
  const segments = path.split('.');
  segments.shift();
  let current: unknown = tokens.color;
  for (const segment of segments) {
    current = (current as Record<string, unknown>)[segment];
  }
  return current as ThemeColor;
}

/** Checks every declared pair in both themes and returns only failures. */
export function checkContrast(tokens: DesignTokens): ContrastViolation[] {
  const violations: ContrastViolation[] = [];
  for (const pair of tokens.contrastPairs) {
    const foreground = colorAt(tokens, pair.foreground);
    const background = colorAt(tokens, pair.background);
    const requiredRatio = pair.largeText === true ? 3 : 4.5;
    for (const theme of ['light', 'dark'] as const) {
      const ratio = contrastRatio(foreground[theme], background[theme]);
      if (ratio < requiredRatio) {
        violations.push({
          foreground: pair.foreground,
          background: pair.background,
          theme,
          ratio,
          requiredRatio,
        });
      }
    }
  }
  return violations;
}

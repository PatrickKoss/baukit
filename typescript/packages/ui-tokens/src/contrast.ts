import type { DesignTokens, ThemeColor } from './schema.js';

export type ThemeName = 'light' | 'dark';

export interface RgbColor {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

export interface ReadableForegroundResult {
  readonly foreground: string;
  readonly ratio: number;
  readonly meetsThreshold: boolean;
}

export interface SemanticContrastRequirement {
  readonly foregroundRole: string;
  readonly backgroundRole: string;
  readonly minimumRatio: number;
}

export interface SemanticContrastViolation {
  readonly foregroundRole: string;
  readonly backgroundRole: string;
  readonly theme: ThemeName;
  readonly achievedRatio: number;
  readonly minimumRatio: number;
}

export interface ContrastViolation {
  readonly foreground: string;
  readonly background: string;
  readonly theme: ThemeName;
  readonly ratio: number;
  readonly requiredRatio: 3 | 4.5;
}

/**
 * Default role paths and contrast rules for product palettes. Products can
 * extend this list or replace it when their semantic token model differs.
 */
export const DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS: readonly SemanticContrastRequirement[] = [
  {
    foregroundRole: 'color.text.primary',
    backgroundRole: 'color.background.primary',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.text.primary',
    backgroundRole: 'color.background.elevated',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.text.muted',
    backgroundRole: 'color.background.primary',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.text.muted',
    backgroundRole: 'color.background.elevated',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.border.primary',
    backgroundRole: 'color.background.primary',
    minimumRatio: 3,
  },
  {
    foregroundRole: 'color.border.primary',
    backgroundRole: 'color.background.elevated',
    minimumRatio: 3,
  },
  {
    foregroundRole: 'color.focus.ring',
    backgroundRole: 'color.background.primary',
    minimumRatio: 3,
  },
  {
    foregroundRole: 'color.focus.ring',
    backgroundRole: 'color.background.elevated',
    minimumRatio: 3,
  },
  {
    foregroundRole: 'color.action.onPrimary',
    backgroundRole: 'color.action.primary',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.action.onSecondary',
    backgroundRole: 'color.action.secondary',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.status.onSuccess',
    backgroundRole: 'color.status.success',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.status.onWarning',
    backgroundRole: 'color.status.warning',
    minimumRatio: 4.5,
  },
  {
    foregroundRole: 'color.status.onDanger',
    backgroundRole: 'color.status.danger',
    minimumRatio: 4.5,
  },
];

/** Normalizes an RGB or RRGGBB hexadecimal color to lowercase #rrggbb. */
export function normalizeHexColor(color: string): string {
  const match = /^#?([\dA-Fa-f]{3}|[\dA-Fa-f]{6})$/u.exec(color);
  if (match?.[1] === undefined) {
    throw new TypeError(
      `Unsupported hexadecimal color ${JSON.stringify(color)}; expected RGB or RRGGBB with an optional #.`,
    );
  }
  const digits = match[1];
  const expanded =
    digits.length === 3
      ? `${digits.charAt(0)}${digits.charAt(0)}${digits.charAt(1)}${digits.charAt(1)}${digits.charAt(2)}${digits.charAt(2)}`
      : digits;
  return `#${expanded.toLowerCase()}`;
}

/** Converts a supported hexadecimal color to integer sRGB channels. */
export function hexToRgb(color: string): RgbColor {
  const normalized = normalizeHexColor(color);
  return {
    r: Number.parseInt(normalized.slice(1, 3), 16),
    g: Number.parseInt(normalized.slice(3, 5), 16),
    b: Number.parseInt(normalized.slice(5, 7), 16),
  };
}

function validateRgbChannel(name: keyof RgbColor, value: number): void {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new RangeError(
      `RGB channel ${name} must be an integer between 0 and 255; received ${String(value)}.`,
    );
  }
}

/** Converts integer sRGB channels to a lowercase six-digit hexadecimal color. */
export function rgbToHex(color: RgbColor): string {
  validateRgbChannel('r', color.r);
  validateRgbChannel('g', color.g);
  validateRgbChannel('b', color.b);
  return `#${[color.r, color.g, color.b]
    .map((value) => value.toString(16).padStart(2, '0'))
    .join('')}`;
}

/** Blends the first opaque color over the second with a ratio clamped to [0, 1]. */
export function blendColors(
  foreground: string,
  background: string,
  foregroundRatio: number,
): string {
  if (!Number.isFinite(foregroundRatio)) {
    throw new RangeError(`Blend ratio must be finite; received ${String(foregroundRatio)}.`);
  }
  const ratio = Math.max(0, Math.min(1, foregroundRatio));
  const foregroundRgb = hexToRgb(foreground);
  const backgroundRgb = hexToRgb(background);
  return rgbToHex({
    r: Math.round(foregroundRgb.r * ratio + backgroundRgb.r * (1 - ratio)),
    g: Math.round(foregroundRgb.g * ratio + backgroundRgb.g * (1 - ratio)),
    b: Math.round(foregroundRgb.b * ratio + backgroundRgb.b * (1 - ratio)),
  });
}

function channel(hex: string, offset: number): number {
  const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.x relative luminance for an opaque hexadecimal sRGB color. */
export function relativeLuminance(color: string): number {
  const hex = normalizeHexColor(color).slice(1);
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

function validateMinimumRatio(minimumRatio: number): void {
  if (!Number.isFinite(minimumRatio) || minimumRatio < 1 || minimumRatio > 21) {
    throw new RangeError(
      `Minimum contrast ratio must be finite and between 1 and 21; received ${String(minimumRatio)}.`,
    );
  }
}

/**
 * Returns the first candidate that meets the threshold. If none does, returns
 * the candidate with the highest ratio and sets `meetsThreshold` to false.
 */
export function chooseReadableForeground(
  background: string,
  candidates: readonly string[],
  minimumRatio: number,
): ReadableForegroundResult {
  validateMinimumRatio(minimumRatio);
  const normalizedBackground = normalizeHexColor(background);
  const normalizedCandidates = candidates.map((candidate) => normalizeHexColor(candidate));
  const firstCandidate = normalizedCandidates[0];
  if (firstCandidate === undefined) {
    throw new RangeError('Readable foreground candidates must contain at least one color.');
  }

  const evaluate = (foreground: string): ReadableForegroundResult => {
    const ratio = contrastRatio(foreground, normalizedBackground);
    return { foreground, ratio, meetsThreshold: ratio >= minimumRatio };
  };

  let best = evaluate(firstCandidate);
  if (best.meetsThreshold) return best;
  for (const candidate of normalizedCandidates.slice(1)) {
    const result = evaluate(candidate);
    if (result.meetsThreshold) return result;
    if (result.ratio > best.ratio) best = result;
  }
  return best;
}

function colorAt(tokens: DesignTokens, path: string): ThemeColor {
  const segments = path.split('.');
  if (segments.shift() !== 'color' || segments.length === 0) {
    throw new TypeError(`Semantic color role ${JSON.stringify(path)} must start with "color.".`);
  }
  let current: unknown = tokens.color;
  for (const segment of segments) {
    if (current === null || typeof current !== 'object' || Array.isArray(current)) {
      throw new TypeError(
        `Semantic color role ${JSON.stringify(path)} does not reference a color token.`,
      );
    }
    current = (current as Record<string, unknown>)[segment];
  }
  if (
    current === null ||
    typeof current !== 'object' ||
    Array.isArray(current) ||
    typeof (current as Record<string, unknown>)['light'] !== 'string' ||
    typeof (current as Record<string, unknown>)['dark'] !== 'string'
  ) {
    throw new TypeError(
      `Semantic color role ${JSON.stringify(path)} does not reference a color token.`,
    );
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

/** Checks every semantic requirement in both themes and returns every failure. */
export function checkSemanticContrastMatrix(
  tokens: DesignTokens,
  requirements: readonly SemanticContrastRequirement[],
): SemanticContrastViolation[] {
  const violations: SemanticContrastViolation[] = [];
  for (const requirement of requirements) {
    validateMinimumRatio(requirement.minimumRatio);
    const foreground = colorAt(tokens, requirement.foregroundRole);
    const background = colorAt(tokens, requirement.backgroundRole);
    for (const theme of ['light', 'dark'] as const) {
      const achievedRatio = contrastRatio(foreground[theme], background[theme]);
      if (achievedRatio < requirement.minimumRatio) {
        violations.push({
          foregroundRole: requirement.foregroundRole,
          backgroundRole: requirement.backgroundRole,
          theme,
          achievedRatio,
          minimumRatio: requirement.minimumRatio,
        });
      }
    }
  }
  return violations;
}

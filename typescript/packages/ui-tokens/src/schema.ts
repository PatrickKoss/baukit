export interface ThemeColor {
  /** A three- or six-digit hexadecimal sRGB color. */
  readonly light: string;
  /** A three- or six-digit hexadecimal sRGB color. */
  readonly dark: string;
}

export interface ColorTokenGroup {
  readonly [name: string]: ColorTokenGroup | ThemeColor;
}

export type TokenScale<T> = Readonly<Record<string, T>>;

export type DimensionValue = number | string;

export interface TypographyTokens {
  readonly family: TokenScale<string>;
  readonly size: TokenScale<DimensionValue>;
  readonly weight: TokenScale<number>;
  readonly lineHeight: TokenScale<DimensionValue>;
}

export interface MotionTokens {
  readonly duration: TokenScale<DimensionValue>;
  readonly easing: TokenScale<string>;
}

export interface ContrastPair {
  /** A complete semantic path such as `color.text.primary`. */
  readonly foreground: string;
  /** A complete semantic path such as `color.background.primary`. */
  readonly background: string;
  /** Large text uses WCAG's 3:1 threshold instead of 4.5:1. */
  readonly largeText?: boolean;
}

/** Cross-platform token source. It intentionally contains no component definitions. */
export interface DesignTokens {
  readonly color: ColorTokenGroup;
  readonly typography: TypographyTokens;
  readonly space: TokenScale<DimensionValue>;
  readonly radius: TokenScale<DimensionValue>;
  readonly motion: MotionTokens;
  readonly elevation: TokenScale<DimensionValue>;
  readonly contrastPairs: readonly ContrastPair[];
}

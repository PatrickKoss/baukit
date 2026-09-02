/**
 * Layout mode arithmetic over a product's own breakpoint numbers. The rules are
 * shared; the numbers are not, so every function takes them as an argument.
 */

export type LayoutMode = 'compact' | 'medium' | 'expanded';

export interface LayoutBreakpoints {
  /** Width at or above which the layout is `medium`. */
  readonly medium: number;
  /** Width at or above which the layout is `expanded`. */
  readonly expanded: number;
}

export type ScreenMaxWidths = Readonly<Record<string, number>>;

export interface ScreenMaxWidthOptions<Widths extends ScreenMaxWidths> {
  readonly maxWidths: Widths;
  /** Key used when a wide screen is requested below the `expanded` mode. */
  readonly narrowFallback: keyof Widths;
  /** Keys that only reach their full width in the `expanded` mode. */
  readonly expandedOnly: readonly (keyof Widths)[];
}

/** Classifies a viewport width. Ties go to the wider mode. */
export function getLayoutMode(width: number, breakpoints: LayoutBreakpoints): LayoutMode {
  if (width >= breakpoints.expanded) return 'expanded';
  if (width >= breakpoints.medium) return 'medium';
  return 'compact';
}

/** Resolves a named content width, narrowing expanded-only keys below that mode. */
export function getScreenMaxWidth<Widths extends ScreenMaxWidths>(
  screen: keyof Widths,
  layoutMode: LayoutMode,
  { maxWidths, narrowFallback, expandedOnly }: ScreenMaxWidthOptions<Widths>,
): number {
  const key = layoutMode !== 'expanded' && expandedOnly.includes(screen) ? narrowFallback : screen;
  const width = maxWidths[key];
  if (width === undefined) {
    throw new Error(`unknown screen max width: ${String(key)}`);
  }
  return width;
}

/** Bottom padding that keeps content clear of a mobile tab bar and the safe area. */
export function getTabContentInset(
  layoutMode: LayoutMode,
  safeAreaBottom: number,
  tabBarHeight: number,
): number {
  if (layoutMode === 'expanded') return 0;
  return tabBarHeight + Math.max(0, safeAreaBottom);
}

function validateNonNegativeDimension(name: string, value: number): void {
  if (!Number.isFinite(value) || value < 0) {
    throw new RangeError(
      `${name} must be a finite non-negative number; received ${String(value)}.`,
    );
  }
}

/** Returns the viewport height left after fixed footer and bottom inset. */
export function getUsableContentHeight(
  viewportHeight: number,
  fixedFooterHeight: number,
  bottomInset: number,
): number {
  validateNonNegativeDimension('viewportHeight', viewportHeight);
  validateNonNegativeDimension('fixedFooterHeight', fixedFooterHeight);
  validateNonNegativeDimension('bottomInset', bottomInset);
  return Math.max(0, viewportHeight - fixedFooterHeight - bottomInset);
}

/** True when the viewport height is at or below the caller's threshold. */
export function isShortViewport(viewportHeight: number, threshold: number): boolean {
  validateNonNegativeDimension('viewportHeight', viewportHeight);
  validateNonNegativeDimension('threshold', threshold);
  return viewportHeight <= threshold;
}

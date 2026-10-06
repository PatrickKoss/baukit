import { getLayoutMode } from '@baukit/ui-tokens';
import type { ReactNode } from 'react';

export interface NavigationSubItem<Icon = unknown> {
  readonly id: string;
  readonly label: string;
  readonly href: string;
  readonly icon?: Icon;
  readonly matches?: (pathname: string) => boolean;
}

export interface NavigationItem<Icon = unknown> extends NavigationSubItem<Icon> {
  readonly icon: Icon;
  readonly children?: readonly NavigationSubItem<Icon>[];
}

export type NavigationProfileMenuEntry = {
  readonly id: string;
  readonly label: string;
  readonly disabled?: boolean;
  readonly tone?: 'default' | 'danger';
} & (
  | {
      readonly href: string;
      readonly matches?: (pathname: string) => boolean;
      readonly onSelect?: never;
    }
  | { readonly onSelect: () => void; readonly href?: never }
);

export interface NavigationAvatarState {
  readonly active: boolean;
  readonly size: number;
}

export type NavigationProfile = {
  readonly label: string;
  readonly subtitle?: string;
  readonly initials: string;
  readonly imageUrl?: string;
  readonly renderAvatar?: (state: NavigationAvatarState) => ReactNode;
} & (
  | { readonly href: string; readonly menu?: never }
  | { readonly menu: readonly NavigationProfileMenuEntry[]; readonly href?: never }
);

export function getNavigationProfileLabel(profile: NavigationProfile): string {
  return profile.subtitle === undefined || profile.subtitle.trim() === ''
    ? profile.label
    : `${profile.label}, ${profile.subtitle}`;
}

export interface ActiveNavigation<Icon = unknown> {
  readonly item: NavigationItem<Icon> | null;
  readonly subItem: NavigationSubItem<Icon> | null;
}

export const NAVIGATION_BAR_DIMENSIONS = {
  avatar: 28,
  icon: 24,
  border: 1,
  targetBorder: 3,
  targetPadding: 8,
  labelFontSize: 11,
  labelLineHeight: 16,
  spacing: 8,
} as const;

export interface NavigationSlotState {
  readonly layout: 'bar' | 'rail';
  readonly collapsed: boolean;
}

export interface NavigationSlots {
  readonly renderBrand?: (state: NavigationSlotState) => ReactNode;
  readonly renderAccessory?: (state: NavigationSlotState) => ReactNode;
  readonly onBarHeightChange?: (height: number) => void;
}

export interface NavigationBarMetrics {
  readonly slotHeight?: number;
  readonly spacing?: number;
  readonly fontSize?: number;
  readonly lineHeight?: number;
}

export function getNavigationBarLineHeight(metrics: NavigationBarMetrics = {}): number {
  return (
    metrics.lineHeight ??
    Math.ceil(
      (metrics.fontSize ?? NAVIGATION_BAR_DIMENSIONS.labelFontSize) *
        (NAVIGATION_BAR_DIMENSIONS.labelLineHeight / NAVIGATION_BAR_DIMENSIONS.labelFontSize),
    )
  );
}

export function getNavigationBarVisualHeight(fontScale = 1): number {
  return Math.max(
    NAVIGATION_BAR_DIMENSIONS.avatar,
    NAVIGATION_BAR_DIMENSIONS.icon * Math.max(1, fontScale),
  );
}

export function getNavigationBarHeight(
  fontScale = 1,
  bottomInset = 0,
  metrics: NavigationBarMetrics = {},
): number {
  const scale = Math.max(1, fontScale);
  return (
    NAVIGATION_BAR_DIMENSIONS.border +
    2 * (NAVIGATION_BAR_DIMENSIONS.targetBorder + NAVIGATION_BAR_DIMENSIONS.targetPadding) +
    getNavigationBarVisualHeight(scale) +
    (metrics.spacing ?? NAVIGATION_BAR_DIMENSIONS.spacing) +
    getNavigationBarLineHeight(metrics) * scale +
    bottomInset +
    (metrics.slotHeight ?? 0)
  );
}

export const NAVIGATION_DIMENSIONS = {
  avatar: NAVIGATION_BAR_DIMENSIONS.avatar,
  rail: 280,
  collapsedRail: 76,
  bar: 64,
  nativeBar: getNavigationBarHeight(),
  target: 44,
  androidTarget: 48,
} as const;

export function getNavigationLayout(width: number): 'bar' | 'rail' {
  return getLayoutMode(width, { medium: 600, expanded: 1024 }) === 'expanded' ? 'rail' : 'bar';
}

function routePath(href: string): string {
  return href.split(/[?#]/, 1)[0] ?? href;
}

export function navigationMatches(item: NavigationSubItem, pathname: string): boolean {
  if (item.matches !== undefined) return item.matches(pathname);
  const path = routePath(pathname);
  const href = routePath(item.href);
  return path === href || (href !== '/' && path.startsWith(`${href}/`));
}

function bestMatch<Item extends NavigationSubItem>(
  items: readonly Item[],
  pathname: string,
): Item | null {
  return items.reduce<Item | null>((best, item) => {
    if (!navigationMatches(item, pathname)) return best;
    return best === null || routePath(item.href).length > routePath(best.href).length ? item : best;
  }, null);
}

export function resolveActiveMenuEntry(
  entries: readonly NavigationProfileMenuEntry[],
  pathname: string,
): NavigationProfileMenuEntry | null {
  return bestMatch(
    entries.filter((entry) => entry.href !== undefined),
    pathname,
  );
}

export function resolveActiveNavigation<Icon>(
  items: readonly NavigationItem<Icon>[],
  pathname: string,
): ActiveNavigation<Icon> {
  const subItem = bestMatch(
    items.flatMap((item) => item.children ?? []),
    pathname,
  );
  const item =
    subItem === null
      ? bestMatch(items, pathname)
      : (items.find((parent) => parent.children?.includes(subItem)) ?? null);
  return { item, subItem };
}

export function nextSectionHref(item: NavigationItem, pathname: string): string {
  const routes = item.children ?? [];
  if (routes.length === 0) return item.href;
  const current = bestMatch(routes, pathname);
  const index = current === null ? -1 : routes.indexOf(current);
  return routes[(index + 1) % routes.length]?.href ?? item.href;
}

export interface NavigationState {
  readonly collapsed: boolean;
  readonly openIds: readonly string[];
}

export type NavigationAction =
  | { readonly type: 'set-collapsed'; readonly collapsed: boolean }
  | { readonly type: 'toggle-group'; readonly id: string; readonly collapsed?: boolean }
  | { readonly type: 'enter-section'; readonly id: string | null };

export function navigationReducer(
  state: NavigationState,
  action: NavigationAction,
): NavigationState {
  if (action.type === 'set-collapsed') return { ...state, collapsed: action.collapsed };
  if (action.type === 'enter-section') {
    return action.id === null || state.openIds.includes(action.id)
      ? state
      : { ...state, openIds: [...state.openIds, action.id] };
  }
  const open = state.openIds.includes(action.id);
  const collapsed = action.collapsed ?? state.collapsed;
  return {
    collapsed: false,
    openIds:
      open && !collapsed
        ? state.openIds.filter((id) => id !== action.id)
        : open
          ? state.openIds
          : [...state.openIds, action.id],
  };
}

export function validateNavigation(
  items: readonly NavigationItem[],
  profile?: NavigationProfile,
): void {
  if (items.length > 5)
    throw new RangeError('Navigation supports at most five main items plus profile.');
  const ids = new Set<string>();
  for (const item of items.flatMap((parent) => [parent, ...(parent.children ?? [])])) {
    if (item.id.trim() === '' || ids.has(item.id))
      throw new Error(`Duplicate or empty navigation id: ${item.id}`);
    ids.add(item.id);
    if (item.label.trim() === '') throw new Error(`Navigation label is empty: ${item.id}`);
    if (item.href.trim() === '') throw new Error(`Navigation href is empty: ${item.id}`);
  }
  if (profile === undefined) return;
  if (profile.label.trim() === '' || profile.initials.trim() === '')
    throw new Error('Profile needs a label and initials.');
  if (profile.href?.trim() === '') throw new Error('Profile href is empty.');
  if (profile.menu !== undefined) {
    if (profile.menu.length === 0) throw new Error('Profile menu is empty.');
    for (const entry of profile.menu) {
      if (entry.id.trim() === '' || ids.has(entry.id))
        throw new Error(`Duplicate or empty navigation id: ${entry.id}`);
      ids.add(entry.id);
      if (entry.label.trim() === '' || entry.href?.trim() === '')
        throw new Error(`Profile entry is empty: ${entry.id}`);
    }
  }
}

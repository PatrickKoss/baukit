import { useCallback, useEffect, useRef, type ReactNode, type RefObject } from 'react';
import {
  ScrollView,
  View,
  useWindowDimensions,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from 'react-native';
import {
  getNavigationBarHeight,
  navigationMatches,
  resolveActiveMenuEntry,
  type NavigationProfile,
} from './index.js';
import {
  Avatar,
  compactVisualStyle,
  follow,
  Target,
  type RovingItemProps,
} from './native-primitives.js';
import { styles } from './native-styles.js';
import { barMetrics, type Navigate, type NavigationTheme } from './native-theme.js';

export interface CompactBarScroll {
  readonly scrollProps: {
    readonly ref: RefObject<ScrollView | null>;
    readonly onLayout: (event: LayoutChangeEvent) => void;
    readonly onContentSizeChange: (width: number) => void;
    readonly onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
  };
  readonly itemLayout: (index: number) => (event: LayoutChangeEvent) => void;
}

interface ProfileProps {
  readonly open: boolean;
  readonly onOpen: () => void;
  readonly triggerRef: RefObject<View | null>;
  readonly id: string;
  readonly profile: NavigationProfile;
  readonly theme: NavigationTheme;
  readonly bar: boolean;
  readonly collapsed: boolean;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly roving: RovingItemProps;
  readonly onLayout?: (event: LayoutChangeEvent) => void;
}
export function profileIsActive(profile: NavigationProfile, pathname: string): boolean {
  return profile.href !== undefined
    ? navigationMatches({ id: 'profile', label: profile.label, href: profile.href }, pathname)
    : resolveActiveMenuEntry(profile.menu, pathname) !== null;
}
export function Profile({
  profile,
  open,
  onOpen,
  triggerRef,
  id,
  theme,
  bar,
  collapsed,
  pathname,
  onNavigate,
  roving,
  onLayout,
}: ProfileProps): ReactNode {
  const { fontScale } = useWindowDimensions();
  const active = profileIsActive(profile, pathname);
  return (
    <View
      style={bar ? styles.barSection : styles.profile}
      testID="navigation-profile"
      onLayout={onLayout}
    >
      <Target
        label={profile.label}
        subtitle={profile.subtitle}
        selected={active}
        theme={theme}
        bar={bar}
        collapsed={collapsed}
        roving={roving}
        triggerRef={triggerRef}
        {...(profile.href === undefined
          ? { expanded: open, controls: id, menu: true }
          : { href: profile.href })}
        onPress={(event) => {
          if (profile.href !== undefined) follow(profile.href, event, onNavigate);
          else onOpen();
        }}
      >
        <Avatar
          profile={profile}
          active={active}
          theme={theme}
          style={bar ? compactVisualStyle(fontScale) : undefined}
        />
      </Target>
    </View>
  );
}

export function useNavigationBarHeight(
  bottomInset = 0,
  theme?: NavigationTheme,
  slotHeight = 0,
): number {
  const { fontScale } = useWindowDimensions();
  return getNavigationBarHeight(fontScale, bottomInset, { ...barMetrics(theme), slotHeight });
}

export function useCompactBarScroll(selectedIndex: number): CompactBarScroll {
  const ref = useRef<ScrollView>(null);
  const viewport = useRef(0);
  const content = useRef(0);
  const offset = useRef(0);
  const frames = useRef(new Map<number, { x: number; width: number }>());
  const revealSelected = useCallback(() => {
    const frame = frames.current.get(selectedIndex);
    if (frame === undefined || viewport.current === 0 || content.current === 0) return;
    if (frame.x >= offset.current && frame.x + frame.width <= offset.current + viewport.current)
      return;
    const x = Math.max(
      0,
      Math.min(frame.x + (frame.width - viewport.current) / 2, content.current - viewport.current),
    );
    ref.current?.scrollTo({ x, animated: false });
    offset.current = x;
  }, [selectedIndex]);
  useEffect(revealSelected, [revealSelected]);
  return {
    scrollProps: {
      ref,
      onLayout: (event: LayoutChangeEvent) => {
        viewport.current = event.nativeEvent.layout.width;
        revealSelected();
      },
      onContentSizeChange: (width: number) => {
        content.current = width;
        revealSelected();
      },
      onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => {
        offset.current = event.nativeEvent.contentOffset.x;
      },
    },
    itemLayout: (index: number) => (event: LayoutChangeEvent) => {
      frames.current.set(index, event.nativeEvent.layout);
      revealSelected();
    },
  };
}

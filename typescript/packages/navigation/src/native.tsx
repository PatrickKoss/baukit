import {
  useEffect,
  useCallback,
  useLayoutEffect,
  useId,
  useRef,
  useState,
  type ComponentPropsWithRef,
  type ReactNode,
} from 'react';
import {
  Image,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  type StyleProp,
  type ViewStyle,
  type TextStyle,
} from 'react-native';
import {
  announce,
  asFocusTarget,
  hostElement,
  type RovingMenuKeyEvent,
  type OverlayBackgroundProps,
  useAriaHiddenInert,
  useOverlayA11y,
  useReducedMotionPreference,
  useRovingMenu,
} from '@baukit/a11y-core';
import {
  getNavigationLayout,
  getNavigationBarHeight,
  getNavigationBarVisualHeight,
  getNavigationBarLineHeight,
  NAVIGATION_BAR_DIMENSIONS,
  type NavigationBarMetrics,
  NAVIGATION_DIMENSIONS,
  navigationMatches,
  nextSectionHref,
  resolveActiveNavigation,
  resolveActiveMenuEntry,
  validateNavigation,
  type NavigationItem,
  type NavigationProfile,
  type NavigationProfileMenuEntry,
} from './index.js';
import { useNavigationState, type CollapseProps } from './use-navigation-state.js';

export interface NavigationIconState {
  readonly active: boolean;
  readonly size: number;
}
export type NavigationIcon = (state: NavigationIconState) => ReactNode;
export interface NavigationTypography {
  readonly fontFamily?: string;
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
  readonly activeBackground: string;
  readonly activeText: string;
  readonly ancestorText: string;
  readonly border: string;
  readonly focus: string;
  readonly radius: number;
  readonly spacing: number;
  readonly typography?: NavigationTypography;
}
function barMetrics(theme?: NavigationTheme): NavigationBarMetrics {
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
function textStyle(theme: NavigationTheme, selected = false, bar = false): TextStyle {
  const typography = theme.typography;
  return {
    fontFamily: typography?.fontFamily,
    fontSize: bar
      ? (typography?.barFontSize ?? NAVIGATION_BAR_DIMENSIONS.labelFontSize)
      : (typography?.fontSize ?? 14),
    fontWeight: selected
      ? (typography?.activeFontWeight ?? '700')
      : (typography?.fontWeight ?? '400'),
    lineHeight: bar ? getNavigationBarLineHeight(barMetrics(theme)) : typography?.lineHeight,
    letterSpacing: typography?.letterSpacing,
  };
}

export interface NavigationInsets {
  readonly top?: number;
  readonly bottom?: number;
  readonly left?: number;
  readonly right?: number;
}
export type Navigate = (href: string) => void;

type FocusablePressableProps = Omit<ComponentPropsWithRef<typeof Pressable>, 'style'> & {
  readonly theme: NavigationTheme;
  readonly selected?: boolean;
  readonly unfocusedBorderColor?: string;
  readonly style?: StyleProp<ViewStyle>;
};
function FocusablePressable({
  theme,
  selected = false,
  unfocusedBorderColor,
  style,
  onFocus,
  onBlur,
  ...props
}: FocusablePressableProps) {
  const [focused, setFocused] = useState(false);
  return (
    <Pressable
      {...props}
      onFocus={(event) => {
        setFocused(true);
        onFocus?.(event);
      }}
      onBlur={(event) => {
        setFocused(false);
        onBlur?.(event);
      }}
      style={[
        style,
        {
          minHeight:
            Platform.OS === 'android'
              ? NAVIGATION_DIMENSIONS.androidTarget
              : NAVIGATION_DIMENSIONS.target,
          minWidth:
            Platform.OS === 'android'
              ? NAVIGATION_DIMENSIONS.androidTarget
              : NAVIGATION_DIMENSIONS.target,
          borderWidth: NAVIGATION_BAR_DIMENSIONS.targetBorder,
          borderColor: focused
            ? selected
              ? theme.activeText
              : theme.focus
            : (unfocusedBorderColor ?? 'transparent'),
        },
      ]}
    />
  );
}

function isModifiedPress(event: GestureResponderEvent): boolean {
  const native: unknown = event.nativeEvent;
  if (typeof native !== 'object' || native === null) return false;
  return (
    ('button' in native && native.button !== undefined && native.button !== 0) ||
    ('metaKey' in native && native.metaKey === true) ||
    ('ctrlKey' in native && native.ctrlKey === true) ||
    ('shiftKey' in native && native.shiftKey === true) ||
    ('altKey' in native && native.altKey === true)
  );
}
function webLink(href: string, active: boolean) {
  return Platform.OS === 'web'
    ? { href, 'aria-current': active ? ('page' as const) : undefined }
    : {};
}
function follow(href: string, event: GestureResponderEvent, onNavigate: Navigate) {
  if (isModifiedPress(event)) return;
  event.preventDefault();
  onNavigate(href);
}

interface TargetProps {
  readonly label: string;
  readonly subtitle?: string | undefined;
  readonly selected?: boolean;
  readonly ancestor?: boolean;
  readonly expanded?: boolean;
  readonly controls?: string;
  readonly menu?: boolean;
  readonly href?: string;
  readonly onPress: (event: GestureResponderEvent) => void;
  readonly theme: NavigationTheme;
  readonly bar?: boolean;
  readonly collapsed?: boolean;
  readonly children: ReactNode;
  readonly roving?: ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;
  readonly triggerRef?: React.RefObject<View | null>;
  readonly testID?: string;
}
function Target({
  label,
  subtitle,
  selected = false,
  ancestor = false,
  expanded,
  controls,
  menu,
  href,
  onPress,
  theme,
  bar = false,
  collapsed = false,
  children,
  roving,
  triggerRef,
  testID,
}: TargetProps) {
  return (
    <FocusablePressable
      theme={theme}
      selected={selected && !ancestor}
      {...{ dataSet: { active: selected ? (ancestor ? 'ancestor' : 'page') : 'false' } }}
      {...roving}
      {...(href === undefined ? {} : webLink(href, selected))}
      {...(Platform.OS === 'web'
        ? {
            title: collapsed ? label : undefined,
            'aria-controls': controls,
            'aria-haspopup': menu ? ('menu' as const) : undefined,
          }
        : {})}
      accessibilityLabel={subtitle === undefined ? label : `${label}, ${subtitle}`}
      accessibilityRole={href === undefined ? 'button' : 'link'}
      accessibilityState={{ selected, ...(expanded === undefined ? {} : { expanded }) }}
      ref={(node) => {
        roving?.ref(node);
        if (triggerRef !== undefined) triggerRef.current = node;
      }}
      onPress={onPress}
      testID={testID}
      style={[
        styles.target,
        bar ? styles.barTarget : styles.railTarget,
        collapsed && styles.collapsedTarget,
        {
          borderRadius: theme.radius,
          gap: theme.spacing,
          backgroundColor: selected && !ancestor ? theme.activeBackground : theme.background,
        },
      ]}
    >
      {children}
      {!collapsed ? (
        <View style={styles.labelGroup}>
          <Text
            numberOfLines={1}
            style={{
              ...textStyle(theme, selected && !ancestor, bar),
              color: ancestor ? theme.ancestorText : selected ? theme.activeText : theme.text,
            }}
          >
            {label}
          </Text>
          {!bar && subtitle !== undefined ? (
            <Text
              numberOfLines={1}
              style={{
                ...textStyle(theme),
                color: theme.muted,
                fontSize: theme.typography?.subtitleFontSize ?? 12,
              }}
            >
              {subtitle}
            </Text>
          ) : null}
        </View>
      ) : null}
    </FocusablePressable>
  );
}
function Decorative({
  children,
  style,
}: {
  readonly children: ReactNode;
  readonly style?: StyleProp<ViewStyle>;
}) {
  return (
    <View
      style={style}
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      aria-hidden
    >
      {children}
    </View>
  );
}

const MENU_WIDTH = 280;
const MENU_EDGE = 8;

interface MenuProps {
  readonly subtitle?: string | undefined;
  readonly renderBackground: (props: OverlayBackgroundProps) => ReactNode;
  readonly entries: readonly NavigationProfileMenuEntry[];
  readonly label: string;
  readonly closeLabel: string;
  readonly pathname: string;
  readonly visible: boolean;
  readonly onClose: () => void;
  readonly onNavigate: Navigate;
  readonly triggerRef: React.RefObject<View | null>;
  readonly theme: NavigationTheme;
  readonly id: string;
}
function Menu({
  subtitle,
  renderBackground,
  entries,
  label,
  closeLabel,
  pathname,
  visible,
  onClose,
  onNavigate,
  triggerRef,
  theme,
  id,
}: MenuProps) {
  const activeEntry = resolveActiveMenuEntry(entries, pathname);
  const containerRef = useRef<View>(null);
  const closeRef = useRef<View>(null);
  const pendingFocus = useRef<(() => void) | null>(null);
  const presented = useRef(false);
  const [webPresented, setWebPresented] = useState(false);
  const [webPosition, setWebPosition] = useState<ViewStyle>();
  const deferFocus = useCallback((task: () => void) => {
    if (presented.current) task();
    else pendingFocus.current = task;
    return {
      cancel: () => {
        pendingFocus.current = null;
        presented.current = false;
      },
    };
  }, []);
  const roving = useRovingMenu({
    active: visible,
    options: entries.map((entry) => ({
      disabled: entry.disabled,
      selected: entry === activeEntry,
    })),
  });
  const { containerProps, backgroundProps } = useOverlayA11y({
    active: visible && (Platform.OS !== 'web' || webPresented),
    deferFocus,
    containerRef,
    triggerRef,
    initialFocusRef: roving.activeIndex === null ? closeRef : roving.initialFocusRef,
    onEscape: onClose,
  });
  const { reducedMotion, resolved } = useReducedMotionPreference();
  const motion = !resolved ? 'unresolved' : reducedMotion ? 'reduced' : 'standard';
  useEffect(() => {
    if (visible) return;
    presented.current = false;
    setWebPresented(false);
  }, [visible]);
  useLayoutEffect(() => {
    if (!visible || Platform.OS !== 'web') return;
    function positionMenu() {
      const trigger = hostElement(triggerRef);
      if (!(trigger instanceof HTMLElement)) return;
      const rect = trigger.getBoundingClientRect();
      const width = Math.min(MENU_WIDTH, window.innerWidth - 2 * MENU_EDGE);
      const below = window.innerHeight - rect.bottom;
      const above = rect.top >= below;
      setWebPosition({
        position: 'absolute',
        left: Math.max(MENU_EDGE, Math.min(rect.left, window.innerWidth - width - MENU_EDGE)),
        width,
        maxHeight: Math.max(
          NAVIGATION_DIMENSIONS.target,
          (above ? rect.top : below) - 2 * MENU_EDGE,
        ),
        ...(above
          ? { bottom: window.innerHeight - rect.top + MENU_EDGE }
          : { top: rect.bottom + MENU_EDGE }),
      });
    }
    positionMenu();
    window.addEventListener('resize', positionMenu);
    window.addEventListener('scroll', positionMenu, true);
    return () => {
      window.removeEventListener('resize', positionMenu);
      window.removeEventListener('scroll', positionMenu, true);
    };
  }, [visible, triggerRef]);
  useEffect(() => {
    if (!visible || Platform.OS !== 'web') return;
    function outside(event: PointerEvent) {
      const menu = hostElement(containerRef);
      const trigger = hostElement(triggerRef);
      if (
        !(event.target instanceof Node) ||
        (menu instanceof Node && menu.contains(event.target)) ||
        (trigger instanceof Node && trigger.contains(event.target))
      )
        return;
      onClose();
    }
    document.addEventListener('pointerdown', outside);
    return () => {
      document.removeEventListener('pointerdown', outside);
    };
  }, [visible, triggerRef, onClose, roving.activeIndex, roving.initialFocusRef]);
  function closeAndRestore() {
    onClose();
    if (Platform.OS === 'web') asFocusTarget(triggerRef)?.focus();
  }
  const menu = (
    <View
      {...containerProps}
      {...{
        onKeyDown: (event: RovingMenuKeyEvent) => {
          if (event.nativeEvent.key === 'Escape') {
            event.preventDefault();
            closeAndRestore();
          }
          if (Platform.OS === 'web' && event.nativeEvent.key === 'Tab') closeAndRestore();
        },
      }}
      ref={containerRef}
      accessibilityViewIsModal={Platform.OS !== 'web'}
      accessibilityLabel={subtitle === undefined ? label : `${label}, ${subtitle}`}
      role="menu"
      nativeID={id}
      testID="navigation-menu"
      style={[
        styles.menu,
        {
          backgroundColor: theme.background,
          borderColor: theme.border,
          borderRadius: theme.radius,
        },
        Platform.OS === 'web' && webPosition,
      ]}
      {...{ dataSet: { motion } }}
    >
      {subtitle === undefined ? null : (
        <Decorative>
          <View style={styles.menuProfile}>
            <Text numberOfLines={1} style={{ ...textStyle(theme), color: theme.text }}>
              {label}
            </Text>
            <Text
              numberOfLines={1}
              style={{
                ...textStyle(theme),
                color: theme.muted,
                fontSize: theme.typography?.subtitleFontSize ?? 12,
              }}
            >
              {subtitle}
            </Text>
          </View>
        </Decorative>
      )}
      <ScrollView>
        {entries.map((entry, index) => (
          <FocusablePressable
            theme={theme}
            selected={entry === activeEntry}
            {...{ dataSet: { active: entry === activeEntry ? 'page' : 'false' } }}
            {...roving.itemProps(index)}
            key={entry.id}
            accessibilityRole="menuitem"
            accessibilityLabel={entry.label}
            accessibilityState={{
              disabled: entry.disabled === true,
              selected: entry === activeEntry,
            }}
            disabled={entry.disabled}
            {...(entry.href === undefined ? {} : webLink(entry.href, entry === activeEntry))}
            onPress={(event) => {
              if (isModifiedPress(event)) return;
              closeAndRestore();
              if (entry.href !== undefined) follow(entry.href, event, onNavigate);
              else entry.onSelect();
            }}
            style={[
              styles.menuItem,
              {
                paddingHorizontal: theme.spacing,
                backgroundColor: entry === activeEntry ? theme.activeBackground : theme.background,
              },
            ]}
          >
            <Text
              style={{
                color: entry === activeEntry ? theme.activeText : theme.text,
                ...textStyle(theme, entry === activeEntry),
              }}
            >
              {entry.label}
            </Text>
          </FocusablePressable>
        ))}
      </ScrollView>
      <FocusablePressable
        theme={theme}
        ref={closeRef}
        accessibilityRole="button"
        accessibilityLabel={closeLabel}
        onPress={closeAndRestore}
        style={styles.menuItem}
      >
        <Text style={{ ...textStyle(theme), color: theme.text }}>{closeLabel}</Text>
      </FocusablePressable>
    </View>
  );
  return (
    <>
      {renderBackground(backgroundProps)}
      <Modal
        visible={visible}
        transparent
        animationType="none"
        onRequestClose={onClose}
        onShow={() => {
          if (!visible) return;
          presented.current = true;
          const focus = pendingFocus.current;
          pendingFocus.current = null;
          focus?.();
          if (Platform.OS === 'web') setWebPresented(true);
        }}
      >
        <View style={styles.modal}>{menu}</View>
      </Modal>
    </>
  );
}

function compactVisualStyle(fontScale: number): ViewStyle {
  return { height: getNavigationBarVisualHeight(fontScale), justifyContent: 'center' };
}

function Avatar({
  profile,
  active,
  theme,
  style,
}: {
  readonly profile: NavigationProfile;
  readonly active: boolean;
  readonly theme: NavigationTheme;
  readonly style?: StyleProp<ViewStyle>;
}) {
  const [failedUrl, setFailedUrl] = useState<string>();
  if (profile.renderAvatar !== undefined) {
    return (
      <Decorative style={style}>
        {profile.renderAvatar({ active, size: NAVIGATION_DIMENSIONS.avatar })}
      </Decorative>
    );
  }
  return (
    <Decorative style={style}>
      {profile.imageUrl !== undefined && failedUrl !== profile.imageUrl ? (
        <Image
          source={{ uri: profile.imageUrl }}
          onError={() => {
            setFailedUrl(profile.imageUrl);
          }}
          style={styles.avatar}
        />
      ) : (
        <View style={[styles.avatar, { backgroundColor: theme.activeBackground }]}>
          <Text allowFontScaling={false} style={{ ...textStyle(theme), color: theme.activeText }}>
            {profile.initials}
          </Text>
        </View>
      )}
    </Decorative>
  );
}

interface ProfileProps {
  readonly open: boolean;
  readonly onOpen: () => void;
  readonly triggerRef: React.RefObject<View | null>;
  readonly id: string;
  readonly profile: NavigationProfile;
  readonly theme: NavigationTheme;
  readonly bar: boolean;
  readonly collapsed: boolean;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly roving: ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;
  readonly onLayout?: (event: LayoutChangeEvent) => void;
}
function profileIsActive(profile: NavigationProfile, pathname: string): boolean {
  return profile.href !== undefined
    ? navigationMatches({ id: 'profile', label: profile.label, href: profile.href }, pathname)
    : resolveActiveMenuEntry(profile.menu, pathname) !== null;
}
function Profile({
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
}: ProfileProps) {
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

export function useNavigationBarHeight(bottomInset = 0, theme?: NavigationTheme): number {
  const { fontScale } = useWindowDimensions();
  return getNavigationBarHeight(fontScale, bottomInset, barMetrics(theme));
}

function useCompactBarScroll(selectedIndex: number) {
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

export interface AppNavigationProps extends CollapseProps {
  readonly items: readonly NavigationItem<NavigationIcon>[];
  readonly profile?: NavigationProfile;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly theme: NavigationTheme;
  readonly width?: number;
  readonly insets?: NavigationInsets;
  readonly label: string;
  readonly collapseLabel: string;
  readonly expandLabel: string;
  readonly closeLabel: string;
  readonly style?: ViewStyle;
}
export function AppNavigation(props: AppNavigationProps) {
  const {
    items,
    profile,
    pathname,
    onNavigate,
    theme,
    insets = {},
    label,
    closeLabel,
    collapseLabel,
    expandLabel,
  } = props;
  const [profileOpen, setProfileOpen] = useState(false);
  const profileTriggerRef = useRef<View>(null);
  const profileMenuId = useId();
  validateNavigation(items, profile);
  useAriaHiddenInert();
  const dimensions = useWindowDimensions();
  const bar = getNavigationLayout(props.width ?? dimensions.width) === 'bar';
  const state = useNavigationState(items, pathname, props);
  const collapsed = !bar && state.collapsed;
  const id = useId();
  const visible = items.flatMap((item) =>
    !bar && !collapsed && state.openIds.includes(item.id)
      ? [item, ...(item.children ?? [])]
      : [item],
  );
  const activeId = visible.some((item) => item.id === state.active.subItem?.id)
    ? state.active.subItem?.id
    : state.active.item?.id;
  const profileActive = profile !== undefined && profileIsActive(profile, pathname);
  const compactScroll = useCompactBarScroll(
    bar ? (profileActive ? items.length : items.findIndex((item) => item.id === activeId)) : -1,
  );
  const roving = useRovingMenu({
    active: true,
    options: [
      ...visible.map((item) => ({
        selected: item.id === activeId,
      })),
      ...(profile === undefined ? [] : [{ selected: profileActive }]),
    ],
  });
  const renderBackground = (backgroundProps: OverlayBackgroundProps = {}) => (
    <View
      {...backgroundProps}
      role="navigation"
      accessibilityLabel={label}
      testID="primary-navigation"
      {...{ dataSet: { layout: bar ? 'bar' : 'rail', collapsed: String(collapsed) } }}
      style={[
        bar ? styles.bar : styles.rail,
        {
          backgroundColor: theme.background,
          borderColor: theme.border,
          paddingBottom: insets.bottom ?? 0,
          ...(bar
            ? {
                minHeight: getNavigationBarHeight(
                  dimensions.fontScale,
                  insets.bottom,
                  barMetrics(theme),
                ),
              }
            : {}),
          paddingLeft: insets.left ?? 0,
          paddingRight: insets.right ?? 0,
        },
        !bar && {
          width: collapsed ? NAVIGATION_DIMENSIONS.collapsedRail : NAVIGATION_DIMENSIONS.rail,
          paddingTop: insets.top ?? 0,
        },
        props.style,
      ]}
    >
      {!bar ? (
        <Target
          label={collapsed ? expandLabel : collapseLabel}
          expanded={!collapsed}
          collapsed
          theme={theme}
          onPress={() => {
            state.setCollapsed(!collapsed);
          }}
        >
          <Decorative>
            <Text style={{ ...textStyle(theme), color: theme.text }}>{collapsed ? '›' : '‹'}</Text>
          </Decorative>
        </Target>
      ) : null}
      <ScrollView
        {...(bar ? compactScroll.scrollProps : {})}
        testID="navigation-items"
        horizontal={bar}
        scrollEnabled
        showsHorizontalScrollIndicator={false}
        style={bar ? styles.barItems : styles.railItems}
        contentContainerStyle={bar ? styles.barContent : undefined}
      >
        {items.map((item, index) => {
          const active = state.active.item?.id === item.id;
          const children = item.children ?? [];
          const open = !collapsed && state.openIds.includes(item.id);
          const href = bar && active ? nextSectionHref(item, pathname) : item.href;
          return (
            <View
              key={item.id}
              style={bar ? styles.barSection : undefined}
              testID={`navigation-section-${item.id}`}
              onLayout={bar ? compactScroll.itemLayout(index) : undefined}
            >
              <Target
                label={item.label}
                selected={active}
                ancestor={active && !bar && !collapsed && state.active.subItem !== null}
                theme={theme}
                bar={bar}
                collapsed={collapsed}
                roving={roving.itemProps(visible.indexOf(item))}
                {...(!bar && children.length > 0
                  ? { expanded: open, controls: `${id}-${String(index)}` }
                  : { href })}
                onPress={(event) => {
                  if (!bar && children.length > 0) state.toggleGroup(item.id);
                  else follow(href, event, onNavigate);
                }}
              >
                <Decorative style={bar ? compactVisualStyle(dimensions.fontScale) : undefined}>
                  {item.icon({ active, size: NAVIGATION_BAR_DIMENSIONS.icon })}
                </Decorative>
              </Target>
              {!bar && open && children.length > 0 ? (
                <View
                  nativeID={`${id}-${String(index)}`}
                  role="group"
                  accessibilityLabel={item.label}
                  style={[styles.children, { borderColor: theme.border }]}
                >
                  {children.map((child) => {
                    const selected = state.active.subItem?.id === child.id;
                    return (
                      <Target
                        key={child.id}
                        label={child.label}
                        selected={selected}
                        href={child.href}
                        theme={theme}
                        roving={roving.itemProps(visible.indexOf(child))}
                        onPress={(event) => {
                          follow(child.href, event, onNavigate);
                        }}
                      >
                        {child.icon === undefined ? null : (
                          <Decorative>{child.icon({ active: selected, size: 20 })}</Decorative>
                        )}
                      </Target>
                    );
                  })}
                </View>
              ) : null}
            </View>
          );
        })}
        {bar && profile !== undefined ? (
          <Profile
            profile={profile}
            open={profileOpen}
            onOpen={() => {
              setProfileOpen(true);
            }}
            triggerRef={profileTriggerRef}
            id={profileMenuId}
            theme={theme}
            bar
            collapsed={false}
            pathname={pathname}
            onNavigate={onNavigate}
            roving={roving.itemProps(visible.length)}
            onLayout={compactScroll.itemLayout(items.length)}
          />
        ) : null}
      </ScrollView>
      {!bar && profile !== undefined ? (
        <Profile
          profile={profile}
          open={profileOpen}
          onOpen={() => {
            setProfileOpen(true);
          }}
          triggerRef={profileTriggerRef}
          id={profileMenuId}
          theme={theme}
          bar={false}
          collapsed={collapsed}
          pathname={pathname}
          onNavigate={onNavigate}
          roving={roving.itemProps(visible.length)}
        />
      ) : null}
    </View>
  );
  if (profile?.menu === undefined) return renderBackground();
  return (
    <Menu
      renderBackground={renderBackground}
      entries={profile.menu}
      closeLabel={closeLabel}
      pathname={pathname}
      visible={profileOpen}
      id={profileMenuId}
      label={profile.label}
      subtitle={profile.subtitle}
      triggerRef={profileTriggerRef}
      theme={theme}
      onClose={() => {
        setProfileOpen(false);
      }}
      onNavigate={onNavigate}
    />
  );
}

export interface SectionPickerProps {
  readonly closeLabel: string;
  readonly item: NavigationItem<NavigationIcon>;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly theme: NavigationTheme;
}
export function SectionPicker({
  item,
  pathname,
  onNavigate,
  theme,
  closeLabel,
}: SectionPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<View>(null);
  const id = useId();
  const active = resolveActiveNavigation([item], pathname).subItem ?? item.children?.[0];
  const entries = item.children ?? [];
  useEffect(() => {
    setOpen(false);
  }, [pathname]);
  if (entries.length === 0) return null;
  const renderBackground = (backgroundProps: OverlayBackgroundProps) => (
    <View {...backgroundProps} role="navigation" accessibilityLabel={item.label}>
      <FocusablePressable
        theme={theme}
        unfocusedBorderColor={theme.border}
        ref={triggerRef}
        {...(Platform.OS === 'web'
          ? { 'aria-controls': id, 'aria-haspopup': 'menu' as const }
          : {})}
        accessibilityRole="button"
        accessibilityLabel={`${item.label}, ${active?.label ?? ''}`}
        accessibilityState={{ expanded: open }}
        onPress={() => {
          setOpen(true);
        }}
        style={[
          styles.picker,
          {
            backgroundColor: theme.background,
            borderColor: theme.border,
            borderRadius: theme.radius,
            padding: theme.spacing,
          },
        ]}
      >
        <View>
          <Text style={{ ...textStyle(theme), color: theme.muted }}>{item.label}</Text>
          <Text style={{ ...textStyle(theme), color: theme.text }}>{active?.label}</Text>
        </View>
        <Decorative>
          <Text style={{ ...textStyle(theme), color: theme.text }}>⌄</Text>
        </Decorative>
      </FocusablePressable>
    </View>
  );
  return (
    <Menu
      renderBackground={renderBackground}
      entries={entries}
      closeLabel={closeLabel}
      pathname={pathname}
      label={item.label}
      visible={open}
      id={id}
      triggerRef={triggerRef}
      theme={theme}
      onClose={() => {
        setOpen(false);
      }}
      onNavigate={(href) => {
        announce(entries.find((entry) => entry.href === href)?.label ?? item.label);
        onNavigate(href);
      }}
    />
  );
}
const styles = StyleSheet.create({
  labelGroup: { minWidth: 0, flexShrink: 1 },
  menuProfile: { padding: 8 },
  target: {
    padding: NAVIGATION_BAR_DIMENSIONS.targetPadding,
  },
  railTarget: { flexDirection: 'row', alignItems: 'center' },
  collapsedTarget: { justifyContent: 'center' },
  barTarget: { flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1 },
  bar: {
    flexDirection: 'row',
    minHeight: NAVIGATION_DIMENSIONS.nativeBar,
    borderTopWidth: NAVIGATION_BAR_DIMENSIONS.border,
    width: '100%',
  },
  rail: { borderRightWidth: 1, height: '100%' },
  railItems: { flex: 1 },
  barItems: { width: '100%' },
  barContent: { flexDirection: 'row', flexGrow: 1, minWidth: '100%' },
  barSection: { flex: 1, minWidth: NAVIGATION_DIMENSIONS.target },
  children: { marginLeft: 20, paddingLeft: 8, borderLeftWidth: 1 },
  profile: { position: 'relative', marginTop: 'auto', paddingTop: 8, borderTopWidth: 1 },
  avatar: {
    width: NAVIGATION_DIMENSIONS.avatar,
    height: NAVIGATION_DIMENSIONS.avatar,
    borderRadius: NAVIGATION_DIMENSIONS.avatar / 2,
    alignItems: 'center',
    justifyContent: 'center',
  },
  modal: { flex: 1, justifyContent: 'flex-end', padding: 8 },
  menu: { borderWidth: 1, padding: 8, maxHeight: '80%' },
  menuItem: {
    justifyContent: 'center',
    padding: 8,
  },
  picker: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    borderWidth: 1,
    width: '100%',
  },
});

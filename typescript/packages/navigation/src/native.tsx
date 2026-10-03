import {
  useEffect,
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
  type StyleProp,
  type ViewStyle,
} from 'react-native';
import {
  announce,
  asFocusTarget,
  hostElement,
  type RovingMenuKeyEvent,
  useAriaHiddenInert,
  useOverlayA11y,
  useReducedMotionPreference,
  useRovingMenu,
} from '@baukit/a11y-core';
import {
  getNavigationLayout,
  NAVIGATION_DIMENSIONS,
  navigationMatches,
  nextSectionHref,
  resolveActiveNavigation,
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
          borderWidth: 3,
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
      accessibilityLabel={label}
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
        <Text
          numberOfLines={1}
          style={{
            color: ancestor ? theme.ancestorText : selected ? theme.activeText : theme.text,
            fontSize: bar ? 11 : 14,
            fontWeight: selected && !ancestor ? '700' : '400',
          }}
        >
          {label}
        </Text>
      ) : null}
    </FocusablePressable>
  );
}
function Decorative({ children }: { readonly children: ReactNode }) {
  return (
    <View
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      aria-hidden
    >
      {children}
    </View>
  );
}

interface MenuProps {
  readonly entries: readonly NavigationProfileMenuEntry[];
  readonly label: string;
  readonly visible: boolean;
  readonly onClose: () => void;
  readonly onNavigate: Navigate;
  readonly triggerRef: React.RefObject<View | null>;
  readonly theme: NavigationTheme;
  readonly id: string;
}
function deferMenuFocus(task: () => void) {
  const frame = requestAnimationFrame(task);
  return {
    cancel: () => {
      cancelAnimationFrame(frame);
    },
  };
}
function Menu({ entries, label, visible, onClose, onNavigate, triggerRef, theme, id }: MenuProps) {
  const containerRef = useRef<View>(null);
  const closeRef = useRef<View>(null);
  const roving = useRovingMenu({
    active: visible,
    options: entries.map((entry) => ({ disabled: entry.disabled })),
  });
  const { containerProps } = useOverlayA11y({
    active: visible && Platform.OS !== 'web',
    deferFocus: deferMenuFocus,
    containerRef,
    triggerRef,
    initialFocusRef: roving.activeIndex === null ? closeRef : roving.initialFocusRef,
    onEscape: onClose,
  });
  const { reducedMotion, resolved } = useReducedMotionPreference();
  const { width } = useWindowDimensions();
  const motion = !resolved ? 'unresolved' : reducedMotion ? 'reduced' : 'standard';
  useEffect(() => {
    if (!visible || Platform.OS !== 'web') return;
    asFocusTarget(roving.activeIndex === null ? closeRef : roving.initialFocusRef)?.focus();
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
      accessibilityLabel={label}
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
        Platform.OS === 'web' && {
          position: 'absolute',
          bottom: '100%',
          right: 0,
          width: Math.min(280, width - 16),
          maxHeight: 400,
          zIndex: 6,
        },
      ]}
      {...{ dataSet: { motion } }}
    >
      <ScrollView>
        {entries.map((entry, index) => (
          <FocusablePressable
            theme={theme}
            {...roving.itemProps(index)}
            key={entry.id}
            accessibilityRole="menuitem"
            accessibilityLabel={entry.label}
            accessibilityState={{ disabled: entry.disabled === true }}
            disabled={entry.disabled}
            {...(entry.href === undefined ? {} : webLink(entry.href, false))}
            onPress={(event) => {
              if (isModifiedPress(event)) return;
              closeAndRestore();
              if (entry.href !== undefined) follow(entry.href, event, onNavigate);
              else entry.onSelect();
            }}
            style={[styles.menuItem, { paddingHorizontal: theme.spacing }]}
          >
            <Text style={{ color: theme.text }}>{entry.label}</Text>
          </FocusablePressable>
        ))}
      </ScrollView>
      <FocusablePressable
        theme={theme}
        ref={closeRef}
        accessibilityRole="button"
        accessibilityLabel={`Close ${label}`}
        onPress={closeAndRestore}
        style={styles.menuItem}
      >
        <Text style={{ color: theme.text }}>Close</Text>
      </FocusablePressable>
    </View>
  );
  if (Platform.OS === 'web') return visible ? menu : null;
  return (
    <Modal visible={visible} transparent animationType="none" onRequestClose={onClose}>
      <View style={styles.modal}>{menu}</View>
    </Modal>
  );
}

interface ProfileProps {
  readonly profile: NavigationProfile;
  readonly theme: NavigationTheme;
  readonly bar: boolean;
  readonly collapsed: boolean;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly roving: ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;
}
function Profile({ profile, theme, bar, collapsed, pathname, onNavigate, roving }: ProfileProps) {
  const [open, setOpen] = useState(false);
  const [failedUrl, setFailedUrl] = useState<string>();
  const triggerRef = useRef<View>(null);
  const id = useId();
  const active =
    profile.href !== undefined
      ? navigationMatches({ id: 'profile', label: profile.label, href: profile.href }, pathname)
      : profile.menu.some(
          (entry) =>
            entry.href !== undefined && navigationMatches({ ...entry, href: entry.href }, pathname),
        );
  return (
    <View style={bar ? styles.barSection : styles.profile} testID="navigation-profile">
      <Target
        label={profile.label}
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
          else setOpen(true);
        }}
      >
        <Decorative>
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
              <Text style={{ color: theme.activeText }}>{profile.initials}</Text>
            </View>
          )}
        </Decorative>
      </Target>
      {profile.menu === undefined ? null : (
        <Menu
          entries={profile.menu}
          visible={open}
          id={id}
          label={profile.label}
          triggerRef={triggerRef}
          theme={theme}
          onClose={() => {
            setOpen(false);
          }}
          onNavigate={onNavigate}
        />
      )}
    </View>
  );
}

export interface AppNavigationProps extends CollapseProps {
  readonly items: readonly NavigationItem<NavigationIcon>[];
  readonly profile?: NavigationProfile;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly theme: NavigationTheme;
  readonly width?: number;
  readonly insets?: NavigationInsets;
  readonly label?: string;
  readonly collapseLabel?: string;
  readonly expandLabel?: string;
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
    label = 'Primary',
    collapseLabel = 'Collapse navigation',
    expandLabel = 'Expand navigation',
  } = props;
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
  const roving = useRovingMenu({
    active: true,
    options: [
      ...visible.map((item) => ({
        selected: item.id === (state.active.subItem?.id ?? state.active.item?.id),
      })),
      ...(profile === undefined ? [] : [{}]),
    ],
  });
  return (
    <View
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
            <Text style={{ color: theme.text }}>{collapsed ? '›' : '‹'}</Text>
          </Decorative>
        </Target>
      ) : null}
      <ScrollView
        horizontal={bar}
        scrollEnabled={!bar}
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
                <Decorative>{item.icon({ active, size: 24 })}</Decorative>
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
            theme={theme}
            bar
            collapsed={false}
            pathname={pathname}
            onNavigate={onNavigate}
            roving={roving.itemProps(visible.length)}
          />
        ) : null}
      </ScrollView>
      {!bar && profile !== undefined ? (
        <Profile
          profile={profile}
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
}

export interface SectionPickerProps {
  readonly item: NavigationItem<NavigationIcon>;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly theme: NavigationTheme;
}
export function SectionPicker({ item, pathname, onNavigate, theme }: SectionPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<View>(null);
  const id = useId();
  const active = resolveActiveNavigation([item], pathname).subItem ?? item.children?.[0];
  const entries = item.children ?? [];
  useEffect(() => {
    setOpen(false);
  }, [pathname]);
  if (entries.length === 0) return null;
  return (
    <View role="navigation" accessibilityLabel={item.label}>
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
          <Text style={{ color: theme.muted }}>{item.label}</Text>
          <Text style={{ color: theme.text }}>{active?.label}</Text>
        </View>
        <Decorative>
          <Text style={{ color: theme.text }}>⌄</Text>
        </Decorative>
      </FocusablePressable>
      <Menu
        entries={entries}
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
    </View>
  );
}
const styles = StyleSheet.create({
  target: {
    minHeight: NAVIGATION_DIMENSIONS.target,
    minWidth: NAVIGATION_DIMENSIONS.target,
    padding: 8,
  },
  railTarget: { flexDirection: 'row', alignItems: 'center' },
  collapsedTarget: { justifyContent: 'center' },
  barTarget: { flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1 },
  bar: {
    flexDirection: 'row',
    minHeight: NAVIGATION_DIMENSIONS.bar,
    borderTopWidth: 1,
    width: '100%',
  },
  rail: { borderRightWidth: 1, height: '100%' },
  railItems: { flex: 1 },
  barItems: { width: '100%' },
  barContent: { flexDirection: 'row', flexGrow: 1 },
  barSection: { flex: 1, minWidth: NAVIGATION_DIMENSIONS.target },
  children: { marginLeft: 20, paddingLeft: 8, borderLeftWidth: 1 },
  profile: { position: 'relative', marginTop: 'auto', paddingTop: 8, borderTopWidth: 1 },
  avatar: {
    width: 28,
    height: 28,
    borderRadius: 14,
    alignItems: 'center',
    justifyContent: 'center',
  },
  modal: { flex: 1, justifyContent: 'flex-end', padding: 8 },
  menu: { borderWidth: 1, padding: 8, maxHeight: '80%' },
  menuItem: {
    minHeight: NAVIGATION_DIMENSIONS.target,
    minWidth: NAVIGATION_DIMENSIONS.target,
    justifyContent: 'center',
    padding: 8,
  },
  picker: {
    minHeight: NAVIGATION_DIMENSIONS.target,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    borderWidth: 1,
    width: '100%',
  },
});

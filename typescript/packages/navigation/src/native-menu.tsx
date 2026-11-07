import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import {
  Modal,
  Platform,
  ScrollView,
  Text,
  View,
  type GestureResponderEvent,
  type ViewStyle,
} from 'react-native';
import {
  asFocusTarget,
  hostElement,
  type OverlayBackgroundProps,
  type RovingMenuKeyEvent,
  useOverlayA11y,
  useReducedMotionPreference,
  useRovingMenu,
} from '@baukit/a11y-core';
import {
  NAVIGATION_DIMENSIONS,
  resolveActiveMenuEntry,
  type NavigationProfileMenuEntry,
} from './index.js';
import {
  Decorative,
  FocusablePressable,
  isModifiedPress,
  webLink,
  type RovingItemProps,
} from './native-primitives.js';
import { styles } from './native-styles.js';
import {
  MENU_EDGE,
  MENU_WIDTH,
  menuModalPadding,
  subtitleTextStyle,
  textStyle,
  type Navigate,
  type NavigationInsets,
  type NavigationTheme,
} from './native-theme.js';

export interface MenuProps {
  readonly subtitle?: string | undefined;
  readonly renderBackground: (props: OverlayBackgroundProps) => ReactNode;
  readonly entries: readonly NavigationProfileMenuEntry[];
  readonly label: string;
  readonly closeLabel: string;
  readonly pathname: string;
  readonly visible: boolean;
  readonly onClose: () => void;
  readonly onNavigate: Navigate;
  readonly triggerRef: RefObject<View | null>;
  readonly theme: NavigationTheme;
  readonly id: string;
  readonly insets?: NavigationInsets | undefined;
}

type Selection = () => void;

/** Holds the chosen entry's action until the closing Modal is gone. */
function usePendingSelection(visible: boolean): {
  readonly isPending: () => boolean;
  readonly begin: (selection: Selection) => void;
  readonly finish: () => void;
} {
  const pending = useRef<Selection | null>(null);
  const finish = useCallback(() => {
    const selection = pending.current;
    pending.current = null;
    selection?.();
  }, []);
  useEffect(() => {
    if (visible) {
      pending.current = null;
      return;
    }
    if (Platform.OS === 'ios' || pending.current === null) return;
    // Android removes an unanimated Modal in the close commit and has no onDismiss event.
    const frame = requestAnimationFrame(finish);
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [visible, finish]);
  useEffect(
    () => () => {
      pending.current = null;
    },
    [],
  );
  const isPending = useCallback(() => pending.current !== null, []);
  const begin = useCallback((selection: Selection) => {
    pending.current = selection;
  }, []);
  return { isPending, begin, finish };
}

interface Presentation {
  readonly webPresented: boolean;
  readonly deferFocus: (task: () => void) => { cancel: () => void };
  readonly onShow: () => void;
}

/** Delays initial focus until the Modal reports it is on screen. */
function usePresentation(visible: boolean): Presentation {
  const pendingFocus = useRef<(() => void) | null>(null);
  const presented = useRef(false);
  const [webPresented, setWebPresented] = useState(false);
  if (!visible && webPresented) setWebPresented(false);
  useEffect(() => {
    if (!visible) presented.current = false;
  }, [visible]);
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
  const onShow = (): void => {
    if (!visible) return;
    presented.current = true;
    const focus = pendingFocus.current;
    pendingFocus.current = null;
    focus?.();
    if (Platform.OS === 'web') setWebPresented(true);
  };
  return { webPresented, deferFocus, onShow };
}

function menuPosition(trigger: HTMLElement): ViewStyle {
  const rect = trigger.getBoundingClientRect();
  const width = Math.min(MENU_WIDTH, window.innerWidth - 2 * MENU_EDGE);
  const below = window.innerHeight - rect.bottom;
  const above = rect.top >= below;
  return {
    position: 'absolute',
    left: Math.max(MENU_EDGE, Math.min(rect.left, window.innerWidth - width - MENU_EDGE)),
    width,
    maxHeight: Math.max(NAVIGATION_DIMENSIONS.target, (above ? rect.top : below) - 2 * MENU_EDGE),
    ...(above
      ? { bottom: window.innerHeight - rect.top + MENU_EDGE }
      : { top: rect.bottom + MENU_EDGE }),
  };
}

function useWebMenuPosition(
  visible: boolean,
  triggerRef: RefObject<View | null>,
): ViewStyle | undefined {
  const [webPosition, setWebPosition] = useState<ViewStyle>();
  useLayoutEffect(() => {
    if (!visible || Platform.OS !== 'web') return;
    function positionMenu(): void {
      const trigger = hostElement(triggerRef);
      if (trigger instanceof HTMLElement) setWebPosition(menuPosition(trigger));
    }
    positionMenu();
    window.addEventListener('resize', positionMenu);
    window.addEventListener('scroll', positionMenu, true);
    return () => {
      window.removeEventListener('resize', positionMenu);
      window.removeEventListener('scroll', positionMenu, true);
    };
  }, [visible, triggerRef]);
  return webPosition;
}

function containsTarget(host: object | null, target: Node): boolean {
  return host instanceof Node && host.contains(target);
}

function useWebOutsidePress(
  visible: boolean,
  refs: { readonly container: RefObject<View | null>; readonly trigger: RefObject<View | null> },
  onClose: () => void,
): void {
  const { container, trigger } = refs;
  useEffect(() => {
    if (!visible || Platform.OS !== 'web') return;
    function outside(event: PointerEvent): void {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (containsTarget(hostElement(container), target)) return;
      if (containsTarget(hostElement(trigger), target)) return;
      onClose();
    }
    document.addEventListener('pointerdown', outside);
    return () => {
      document.removeEventListener('pointerdown', outside);
    };
  }, [visible, container, trigger, onClose]);
}

function MenuHeader({
  label,
  subtitle,
  theme,
}: {
  readonly label: string;
  readonly subtitle: string;
  readonly theme: NavigationTheme;
}): ReactNode {
  return (
    <Decorative>
      <View style={styles.menuProfile}>
        <Text numberOfLines={1} style={{ ...textStyle(theme), color: theme.text }}>
          {label}
        </Text>
        <Text numberOfLines={1} style={subtitleTextStyle(theme)}>
          {subtitle}
        </Text>
      </View>
    </Decorative>
  );
}

function entryTextColor(
  theme: NavigationTheme,
  entry: NavigationProfileMenuEntry,
  active: boolean,
): string {
  if (entry.tone === 'danger') return theme.danger;
  return active ? theme.activeText : theme.text;
}

interface MenuEntryProps {
  readonly entry: NavigationProfileMenuEntry;
  readonly active: boolean;
  readonly roving: RovingItemProps;
  readonly theme: NavigationTheme;
  readonly onSelect: (entry: NavigationProfileMenuEntry, event: GestureResponderEvent) => void;
}
function MenuEntry({ entry, active, roving, theme, onSelect }: MenuEntryProps): ReactNode {
  return (
    <FocusablePressable
      theme={theme}
      selected={active}
      {...{ dataSet: { active: active ? 'page' : 'false', tone: entry.tone } }}
      {...roving}
      accessibilityRole="menuitem"
      accessibilityLabel={entry.label}
      accessibilityState={{ disabled: entry.disabled === true, selected: active }}
      disabled={entry.disabled}
      {...(entry.href === undefined ? {} : webLink(entry.href, active))}
      onPress={(event) => {
        onSelect(entry, event);
      }}
      style={[
        styles.menuItem,
        {
          paddingHorizontal: theme.spacing,
          backgroundColor:
            active && entry.tone !== 'danger' ? theme.activeBackground : theme.background,
        },
      ]}
    >
      <Text style={{ color: entryTextColor(theme, entry, active), ...textStyle(theme, active) }}>
        {entry.label}
      </Text>
    </FocusablePressable>
  );
}

function selectionFor(entry: NavigationProfileMenuEntry, onNavigate: Navigate): Selection {
  const href = entry.href;
  if (href === undefined) return entry.onSelect;
  return () => {
    onNavigate(href);
  };
}

function motionState(resolved: boolean, reducedMotion: boolean): string {
  if (!resolved) return 'unresolved';
  return reducedMotion ? 'reduced' : 'standard';
}

interface MenuPanelProps {
  readonly menu: MenuProps;
  readonly containerRef: RefObject<View | null>;
  readonly closeRef: RefObject<View | null>;
  readonly containerProps: object;
  readonly roving: ReturnType<typeof useRovingMenu>;
  readonly activeEntry: NavigationProfileMenuEntry | null;
  readonly webPosition: ViewStyle | undefined;
  readonly onCloseAndRestore: () => void;
  readonly onSelect: MenuEntryProps['onSelect'];
}

function MenuPanel({
  menu,
  containerRef,
  closeRef,
  containerProps,
  roving,
  activeEntry,
  webPosition,
  onCloseAndRestore,
  onSelect,
}: MenuPanelProps): ReactNode {
  const { theme, subtitle, label, closeLabel } = menu;
  const { reducedMotion, resolved } = useReducedMotionPreference();
  return (
    <View
      {...containerProps}
      {...{
        onKeyDown: (event: RovingMenuKeyEvent) => {
          if (event.nativeEvent.key === 'Escape') {
            event.preventDefault();
            onCloseAndRestore();
          }
          if (Platform.OS === 'web' && event.nativeEvent.key === 'Tab') onCloseAndRestore();
        },
      }}
      ref={containerRef}
      accessibilityViewIsModal={Platform.OS !== 'web'}
      accessibilityLabel={subtitle === undefined ? label : `${label}, ${subtitle}`}
      role="menu"
      nativeID={menu.id}
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
      {...{ dataSet: { motion: motionState(resolved, reducedMotion) } }}
    >
      {subtitle === undefined ? null : (
        <MenuHeader label={label} subtitle={subtitle} theme={theme} />
      )}
      <ScrollView>
        {menu.entries.map((entry, index) => (
          <MenuEntry
            key={entry.id}
            entry={entry}
            active={entry === activeEntry}
            roving={roving.itemProps(index)}
            theme={theme}
            onSelect={onSelect}
          />
        ))}
      </ScrollView>
      <FocusablePressable
        theme={theme}
        ref={closeRef}
        accessibilityRole="button"
        accessibilityLabel={closeLabel}
        onPress={onCloseAndRestore}
        style={styles.menuItem}
      >
        <Text style={{ ...textStyle(theme), color: theme.text }}>{closeLabel}</Text>
      </FocusablePressable>
    </View>
  );
}

export function Menu(props: MenuProps): ReactNode {
  const { entries, visible, onClose, triggerRef } = props;
  const activeEntry = resolveActiveMenuEntry(entries, props.pathname);
  const containerRef = useRef<View>(null);
  const closeRef = useRef<View>(null);
  const selection = usePendingSelection(visible);
  const presentation = usePresentation(visible);
  const roving = useRovingMenu({
    active: visible,
    options: entries.map((entry) => ({
      disabled: entry.disabled,
      selected: entry === activeEntry,
    })),
  });
  const { containerProps, backgroundProps } = useOverlayA11y({
    active: visible && (Platform.OS !== 'web' || presentation.webPresented),
    deferFocus: presentation.deferFocus,
    containerRef,
    triggerRef,
    initialFocusRef: roving.activeIndex === null ? closeRef : roving.initialFocusRef,
    onEscape: onClose,
  });
  const webPosition = useWebMenuPosition(visible, triggerRef);
  useWebOutsidePress(visible, { container: containerRef, trigger: triggerRef }, onClose);
  function closeAndRestore(): void {
    onClose();
    if (Platform.OS === 'web') asFocusTarget(triggerRef)?.focus();
  }
  function select(entry: NavigationProfileMenuEntry, event: GestureResponderEvent): void {
    if (isModifiedPress(event) || selection.isPending()) return;
    event.preventDefault();
    selection.begin(selectionFor(entry, props.onNavigate));
    closeAndRestore();
  }
  return (
    <>
      {props.renderBackground(backgroundProps)}
      <Modal
        visible={visible}
        transparent
        animationType="none"
        onRequestClose={onClose}
        onDismiss={() => {
          if (!visible) selection.finish();
        }}
        onShow={presentation.onShow}
      >
        <View style={[styles.modal, menuModalPadding(props.insets)]}>
          <MenuPanel
            menu={props}
            containerRef={containerRef}
            closeRef={closeRef}
            containerProps={containerProps}
            roving={roving}
            activeEntry={activeEntry}
            webPosition={webPosition}
            onCloseAndRestore={closeAndRestore}
            onSelect={select}
          />
        </View>
      </Modal>
    </>
  );
}

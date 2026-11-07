import { useState, type ComponentPropsWithRef, type ReactNode, type RefObject } from 'react';
import {
  Image,
  Platform,
  Pressable,
  Text,
  View,
  type GestureResponderEvent,
  type NativeSyntheticEvent,
  type StyleProp,
  type ViewStyle,
} from 'react-native';
import type { useRovingMenu } from '@baukit/a11y-core';
import {
  getNavigationBarVisualHeight,
  NAVIGATION_BAR_DIMENSIONS,
  NAVIGATION_DIMENSIONS,
  type NavigationProfile,
} from './index.js';
import { styles } from './native-styles.js';
import {
  subtitleTextStyle,
  textStyle,
  type Navigate,
  type NavigationTheme,
} from './native-theme.js';

export type RovingItemProps = ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;

type FocusablePressableProps = Omit<ComponentPropsWithRef<typeof Pressable>, 'style'> & {
  readonly theme: NavigationTheme;
  readonly selected?: boolean;
  readonly unfocusedBorderColor?: string;
  readonly style?: StyleProp<ViewStyle>;
};
function showsFocusRing(event: NativeSyntheticEvent<unknown>): boolean {
  if (Platform.OS !== 'web') return true;
  const target: unknown = (event.nativeEvent as { target?: unknown } | undefined)?.target;
  return !(target instanceof Element) || target.matches(':focus-visible');
}
function focusBorderColor(
  theme: NavigationTheme,
  focused: boolean,
  selected: boolean,
  unfocused: string | undefined,
): string {
  if (!focused) return unfocused ?? 'transparent';
  return selected ? theme.activeText : theme.focus;
}
function targetSize(): number {
  return Platform.OS === 'android'
    ? NAVIGATION_DIMENSIONS.androidTarget
    : NAVIGATION_DIMENSIONS.target;
}

export function FocusablePressable({
  theme,
  selected = false,
  unfocusedBorderColor,
  style,
  onFocus,
  onBlur,
  ...props
}: FocusablePressableProps): ReactNode {
  const [focused, setFocused] = useState(false);
  const size = targetSize();
  return (
    <Pressable
      {...props}
      onFocus={(event) => {
        setFocused(showsFocusRing(event));
        onFocus?.(event);
      }}
      onBlur={(event) => {
        setFocused(false);
        onBlur?.(event);
      }}
      style={[
        style,
        {
          minHeight: size,
          minWidth: size,
          borderWidth: NAVIGATION_BAR_DIMENSIONS.targetBorder,
          borderColor: focusBorderColor(theme, focused, selected, unfocusedBorderColor),
        },
      ]}
    />
  );
}

export function isModifiedPress(event: GestureResponderEvent): boolean {
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
export function webLink(
  href: string,
  active: boolean,
): { href?: string; 'aria-current'?: 'page' | undefined } {
  return Platform.OS === 'web'
    ? { href, 'aria-current': active ? ('page' as const) : undefined }
    : {};
}
export function follow(href: string, event: GestureResponderEvent, onNavigate: Navigate): void {
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
  readonly roving?: RovingItemProps;
  readonly triggerRef?: RefObject<View | null>;
  readonly testID?: string;
}

function activeMarker(selected: boolean, ancestor: boolean): 'ancestor' | 'page' | 'false' {
  if (!selected) return 'false';
  return ancestor ? 'ancestor' : 'page';
}

function webTargetProps(
  label: string,
  {
    collapsed = false,
    controls,
    menu = false,
  }: Pick<TargetProps, 'collapsed' | 'controls' | 'menu'>,
): Record<string, string | undefined> {
  if (Platform.OS !== 'web') return {};
  return {
    title: collapsed ? label : undefined,
    'aria-controls': controls,
    'aria-haspopup': menu ? 'menu' : undefined,
  };
}

function labelColor(theme: NavigationTheme, selected: boolean, ancestor: boolean): string {
  if (ancestor) return theme.ancestorText;
  return selected ? theme.activeText : theme.text;
}

interface TargetLabelProps {
  readonly label: string;
  readonly subtitle: string | undefined;
  readonly selected: boolean;
  readonly ancestor: boolean;
  readonly bar: boolean;
  readonly theme: NavigationTheme;
}
function TargetLabel({
  label,
  subtitle,
  selected,
  ancestor,
  bar,
  theme,
}: TargetLabelProps): ReactNode {
  return (
    <View style={styles.labelGroup}>
      <Text
        numberOfLines={1}
        style={{
          ...textStyle(theme, selected && !ancestor, bar),
          color: labelColor(theme, selected, ancestor),
        }}
      >
        {label}
      </Text>
      {!bar && subtitle !== undefined ? (
        <Text numberOfLines={1} style={subtitleTextStyle(theme)}>
          {subtitle}
        </Text>
      ) : null}
    </View>
  );
}

export function Target(props: TargetProps): ReactNode {
  const { label, subtitle, href, theme, roving, triggerRef, children } = props;
  const selected = props.selected ?? false;
  const ancestor = props.ancestor ?? false;
  const bar = props.bar ?? false;
  const collapsed = props.collapsed ?? false;
  const expanded = props.expanded;
  return (
    <FocusablePressable
      theme={theme}
      selected={selected && !ancestor}
      {...{ dataSet: { active: activeMarker(selected, ancestor) } }}
      {...roving}
      {...(href === undefined ? {} : webLink(href, selected))}
      {...webTargetProps(label, props)}
      accessibilityLabel={subtitle === undefined ? label : `${label}, ${subtitle}`}
      accessibilityRole={href === undefined ? 'button' : 'link'}
      accessibilityState={{ selected, ...(expanded === undefined ? {} : { expanded }) }}
      ref={(node) => {
        roving?.ref(node);
        if (triggerRef !== undefined) triggerRef.current = node;
      }}
      onPress={props.onPress}
      testID={props.testID}
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
        <TargetLabel
          label={label}
          subtitle={subtitle}
          selected={selected}
          ancestor={ancestor}
          bar={bar}
          theme={theme}
        />
      ) : null}
    </FocusablePressable>
  );
}
export function Decorative({
  children,
  style,
}: {
  readonly children: ReactNode;
  readonly style?: StyleProp<ViewStyle>;
}): ReactNode {
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

export function compactVisualStyle(fontScale: number): ViewStyle {
  return { height: getNavigationBarVisualHeight(fontScale), justifyContent: 'center' };
}

export function Avatar({
  profile,
  active,
  theme,
  style,
}: {
  readonly profile: NavigationProfile;
  readonly active: boolean;
  readonly theme: NavigationTheme;
  readonly style?: StyleProp<ViewStyle>;
}): ReactNode {
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

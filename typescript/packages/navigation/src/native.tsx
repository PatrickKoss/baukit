import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from 'react';
import {
  ScrollView,
  Text,
  View,
  useWindowDimensions,
  type StyleProp,
  type ViewStyle,
} from 'react-native';
import { type OverlayBackgroundProps, useAriaHiddenInert, useRovingMenu } from '@baukit/a11y-core';
import {
  getNavigationLayout,
  getNavigationBarHeight,
  NAVIGATION_BAR_DIMENSIONS,
  type NavigationSlots,
  NAVIGATION_DIMENSIONS,
  nextSectionHref,
  validateNavigation,
  type NavigationItem,
  type NavigationProfile,
  type NavigationSubItem,
} from './index.js';
import { Menu } from './native-menu.js';
import { compactVisualStyle, Decorative, follow, Target } from './native-primitives.js';
import {
  Profile,
  profileIsActive,
  useCompactBarScroll,
  type CompactBarScroll,
} from './native-profile.js';
import { styles } from './native-styles.js';
import {
  barMetrics,
  textStyle,
  type Navigate,
  type NavigationIcon,
  type NavigationInsets,
  type NavigationTheme,
} from './native-theme.js';
import {
  useNavigationState,
  type CollapseProps,
  type NavigationStateHandle,
} from './use-navigation-state.js';

export type {
  Navigate,
  NavigationIcon,
  NavigationIconState,
  NavigationInsets,
  NavigationTheme,
  NavigationTypography,
} from './native-theme.js';
export { useNavigationBarHeight } from './native-profile.js';

export interface AppNavigationProps extends CollapseProps, NavigationSlots {
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

const CHILD_ICON_SIZE = 20;

interface SlotContent {
  readonly brand: ReactNode;
  readonly accessory: ReactNode;
  readonly hasBrand: boolean;
  readonly hasAccessory: boolean;
}

function rendersSomething(node: ReactNode): boolean {
  return node !== null && node !== undefined && typeof node !== 'boolean';
}

function useSlotContent(props: AppNavigationProps, bar: boolean, collapsed: boolean): SlotContent {
  const slotState = { layout: bar ? ('bar' as const) : ('rail' as const), collapsed };
  const brand = props.renderBrand?.(slotState);
  const accessory = props.renderAccessory?.(slotState);
  return {
    brand,
    accessory,
    hasBrand: rendersSomething(brand),
    hasAccessory: rendersSomething(accessory),
  };
}

function useBarHeightReport(
  bar: boolean,
  barHeight: number,
  onBarHeightChange: ((height: number) => void) | undefined,
): void {
  useEffect(() => {
    if (bar) onBarHeightChange?.(barHeight);
  }, [bar, barHeight, onBarHeightChange]);
}

interface NavigationModel {
  readonly props: AppNavigationProps;
  readonly insets: NavigationInsets;
  readonly bar: boolean;
  readonly collapsed: boolean;
  readonly fontScale: number;
  readonly barHeight: number;
  readonly state: NavigationStateHandle<NavigationIcon>;
  readonly id: string;
  readonly visible: readonly (NavigationItem<NavigationIcon> | NavigationSubItem<NavigationIcon>)[];
  readonly roving: ReturnType<typeof useRovingMenu>;
  readonly compactScroll: CompactBarScroll;
  readonly slots: SlotContent;
  readonly setSlotHeight: (height: number) => void;
  readonly profileOpen: boolean;
  readonly setProfileOpen: (open: boolean) => void;
  readonly profileTriggerRef: RefObject<View | null>;
  readonly profileMenuId: string;
}

function visibleItems(
  items: readonly NavigationItem<NavigationIcon>[],
  expanded: boolean,
  openIds: readonly string[],
): (NavigationItem<NavigationIcon> | NavigationSubItem<NavigationIcon>)[] {
  return items.flatMap((item) =>
    expanded && openIds.includes(item.id) ? [item, ...(item.children ?? [])] : [item],
  );
}

function compactSelectedIndex(
  items: readonly NavigationItem<NavigationIcon>[],
  activeId: string | undefined,
  profileActive: boolean,
): number {
  return profileActive ? items.length : items.findIndex((item) => item.id === activeId);
}

function useAppNavigationModel(props: AppNavigationProps): NavigationModel {
  const { items, profile, pathname, theme, insets = {} } = props;
  const [slotHeight, setSlotHeight] = useState(0);
  const [profileOpen, setProfileOpen] = useState(false);
  const profileTriggerRef = useRef<View>(null);
  const profileMenuId = useId();
  validateNavigation(items, profile);
  useAriaHiddenInert();
  const dimensions = useWindowDimensions();
  const bar = getNavigationLayout(props.width ?? dimensions.width) === 'bar';
  const state = useNavigationState(items, pathname, props);
  const collapsed = !bar && state.collapsed;
  const slots = useSlotContent(props, bar, collapsed);
  const barHeight = getNavigationBarHeight(dimensions.fontScale, insets.bottom, {
    ...barMetrics(theme),
    slotHeight: slots.hasBrand || slots.hasAccessory ? slotHeight : 0,
  });
  useBarHeightReport(bar, barHeight, props.onBarHeightChange);
  const id = useId();
  const visible = visibleItems(items, !bar && !collapsed, state.openIds);
  const activeId = visible.some((item) => item.id === state.active.subItem?.id)
    ? state.active.subItem?.id
    : state.active.item?.id;
  const profileActive = profile !== undefined && profileIsActive(profile, pathname);
  const compactScroll = useCompactBarScroll(
    bar ? compactSelectedIndex(items, activeId, profileActive) : -1,
  );
  const roving = useRovingMenu({
    active: true,
    options: [
      ...visible.map((item) => ({ selected: item.id === activeId })),
      ...(profile === undefined ? [] : [{ selected: profileActive }]),
    ],
  });
  return {
    props,
    insets,
    bar,
    collapsed,
    fontScale: dimensions.fontScale,
    barHeight,
    state,
    id,
    visible,
    roving,
    compactScroll,
    slots,
    setSlotHeight,
    profileOpen,
    setProfileOpen,
    profileTriggerRef,
    profileMenuId,
  };
}

function SlotsView({ model }: { readonly model: NavigationModel }): ReactNode {
  const { bar, slots, props } = model;
  return (
    <View
      testID="navigation-slots"
      style={[
        styles.slots,
        bar && styles.barSlots,
        { padding: props.theme.spacing, gap: props.theme.spacing },
      ]}
      onLayout={(event) => {
        if (bar) model.setSlotHeight(event.nativeEvent.layout.height);
      }}
    >
      {!slots.hasBrand ? null : (
        <View testID="navigation-brand" style={[styles.slot, bar && styles.barBrand]}>
          {slots.brand}
        </View>
      )}
      {!slots.hasAccessory ? null : (
        <View testID="navigation-accessory" style={styles.slot}>
          {slots.accessory}
        </View>
      )}
    </View>
  );
}

function CollapseToggle({ model }: { readonly model: NavigationModel }): ReactNode {
  const { collapsed, props } = model;
  return (
    <Target
      label={collapsed ? props.expandLabel : props.collapseLabel}
      expanded={!collapsed}
      collapsed
      theme={props.theme}
      onPress={() => {
        model.state.setCollapsed(!collapsed);
      }}
    >
      <Decorative>
        <Text style={{ ...textStyle(props.theme), color: props.theme.text }}>
          {collapsed ? '›' : '‹'}
        </Text>
      </Decorative>
    </Target>
  );
}

function SectionChildren({
  model,
  item,
  groupId,
}: {
  readonly model: NavigationModel;
  readonly item: NavigationItem<NavigationIcon>;
  readonly groupId: string;
}): ReactNode {
  const { state, roving, visible, props } = model;
  return (
    <View
      nativeID={groupId}
      role="group"
      accessibilityLabel={item.label}
      style={[styles.children, { borderColor: props.theme.border }]}
    >
      {(item.children ?? []).map((child) => {
        const selected = state.active.subItem?.id === child.id;
        return (
          <Target
            key={child.id}
            label={child.label}
            selected={selected}
            href={child.href}
            theme={props.theme}
            roving={roving.itemProps(visible.indexOf(child))}
            onPress={(event) => {
              follow(child.href, event, props.onNavigate);
            }}
          >
            {child.icon === undefined ? null : (
              <Decorative>{child.icon({ active: selected, size: CHILD_ICON_SIZE })}</Decorative>
            )}
          </Target>
        );
      })}
    </View>
  );
}

function sectionHref(
  item: NavigationItem<NavigationIcon>,
  cycle: boolean,
  pathname: string,
): string {
  return cycle ? nextSectionHref(item, pathname) : item.href;
}

function NavigationSection({
  model,
  item,
  index,
}: {
  readonly model: NavigationModel;
  readonly item: NavigationItem<NavigationIcon>;
  readonly index: number;
}): ReactNode {
  const { bar, collapsed, state, props } = model;
  const active = state.active.item?.id === item.id;
  const group = !bar && (item.children ?? []).length > 0;
  const open = !collapsed && state.openIds.includes(item.id);
  const href = sectionHref(item, bar && active, props.pathname);
  const groupId = `${model.id}-${String(index)}`;
  return (
    <View
      style={bar ? styles.barSection : undefined}
      testID={`navigation-section-${item.id}`}
      onLayout={bar ? model.compactScroll.itemLayout(index) : undefined}
    >
      <Target
        label={item.label}
        selected={active}
        ancestor={active && !bar && !collapsed && state.active.subItem !== null}
        theme={props.theme}
        bar={bar}
        collapsed={collapsed}
        roving={model.roving.itemProps(model.visible.indexOf(item))}
        {...(group ? { expanded: open, controls: groupId } : { href })}
        onPress={(event) => {
          if (group) state.toggleGroup(item.id);
          else follow(href, event, props.onNavigate);
        }}
      >
        <Decorative style={bar ? compactVisualStyle(model.fontScale) : undefined}>
          {item.icon({ active, size: NAVIGATION_BAR_DIMENSIONS.icon })}
        </Decorative>
      </Target>
      {group && open ? <SectionChildren model={model} item={item} groupId={groupId} /> : null}
    </View>
  );
}

function ProfileTarget({
  model,
  profile,
}: {
  readonly model: NavigationModel;
  readonly profile: NavigationProfile;
}): ReactNode {
  const { bar, props } = model;
  return (
    <Profile
      profile={profile}
      open={model.profileOpen}
      onOpen={() => {
        model.setProfileOpen(true);
      }}
      triggerRef={model.profileTriggerRef}
      id={model.profileMenuId}
      theme={props.theme}
      bar={bar}
      collapsed={model.collapsed}
      pathname={props.pathname}
      onNavigate={props.onNavigate}
      roving={model.roving.itemProps(model.visible.length)}
      {...(bar ? { onLayout: model.compactScroll.itemLayout(props.items.length) } : {})}
    />
  );
}

function surfaceStyle(model: NavigationModel): ViewStyle {
  const { bar, collapsed, insets, props } = model;
  return {
    backgroundColor: props.theme.background,
    borderColor: props.theme.border,
    paddingBottom: insets.bottom ?? 0,
    ...(bar ? { minHeight: model.barHeight } : {}),
    paddingLeft: insets.left ?? 0,
    paddingRight: insets.right ?? 0,
    ...(bar
      ? {}
      : {
          width: collapsed ? NAVIGATION_DIMENSIONS.collapsedRail : NAVIGATION_DIMENSIONS.rail,
          paddingTop: insets.top ?? 0,
        }),
  };
}

function itemsStyle(model: NavigationModel): StyleProp<ViewStyle> {
  if (!model.bar) return styles.railItems;
  const height =
    getNavigationBarHeight(model.fontScale, 0, barMetrics(model.props.theme)) -
    NAVIGATION_BAR_DIMENSIONS.border;
  return [styles.barItems, { height }];
}

function NavigationSurface({
  model,
  backgroundProps,
}: {
  readonly model: NavigationModel;
  readonly backgroundProps: OverlayBackgroundProps;
}): ReactNode {
  const { bar, collapsed, props, slots } = model;
  const profile = props.profile;
  return (
    <View
      {...backgroundProps}
      role="navigation"
      accessibilityLabel={props.label}
      testID="primary-navigation"
      {...{ dataSet: { layout: bar ? 'bar' : 'rail', collapsed: String(collapsed) } }}
      style={[bar ? styles.bar : styles.rail, surfaceStyle(model), props.style]}
    >
      {slots.hasBrand || slots.hasAccessory ? <SlotsView model={model} /> : null}
      {!bar ? <CollapseToggle model={model} /> : null}
      <ScrollView
        {...(bar ? model.compactScroll.scrollProps : {})}
        testID="navigation-items"
        horizontal={bar}
        scrollEnabled
        showsHorizontalScrollIndicator={false}
        style={itemsStyle(model)}
        contentContainerStyle={bar ? styles.barContent : undefined}
      >
        {props.items.map((item, index) => (
          <NavigationSection key={item.id} model={model} item={item} index={index} />
        ))}
        {bar && profile !== undefined ? <ProfileTarget model={model} profile={profile} /> : null}
      </ScrollView>
      {!bar && profile !== undefined ? <ProfileTarget model={model} profile={profile} /> : null}
    </View>
  );
}

export function AppNavigation(props: AppNavigationProps): ReactNode {
  const model = useAppNavigationModel(props);
  const renderBackground = (backgroundProps: OverlayBackgroundProps = {}): ReactNode => (
    <NavigationSurface model={model} backgroundProps={backgroundProps} />
  );
  const profile = props.profile;
  if (profile?.menu === undefined) return renderBackground();
  return (
    <Menu
      renderBackground={renderBackground}
      entries={profile.menu}
      closeLabel={props.closeLabel}
      pathname={props.pathname}
      visible={model.profileOpen}
      id={model.profileMenuId}
      label={profile.label}
      subtitle={profile.subtitle}
      triggerRef={model.profileTriggerRef}
      theme={props.theme}
      insets={model.insets}
      onClose={() => {
        model.setProfileOpen(false);
      }}
      onNavigate={props.onNavigate}
    />
  );
}

export { SectionPicker, type SectionPickerProps } from './native-section-picker.js';

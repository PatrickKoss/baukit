import {
  useId,
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
  type RefObject,
} from 'react';
import {
  useAriaHiddenInert,
  useReducedMotionPreference,
  useRovingMenu,
} from '@baukit/a11y-core/web';
import {
  getNavigationLayout,
  NAVIGATION_DIMENSIONS,
  nextSectionHref,
  validateNavigation,
  type NavigationItem,
  type NavigationSlots,
  type NavigationProfile,
  type NavigationSubItem,
} from './index.js';
import {
  useNavigationState,
  type CollapseProps,
  type NavigationStateHandle,
} from './use-navigation-state.js';
import {
  follow,
  Link,
  type Navigate,
  type NavigationIcon,
  type NavigationLinkRenderer,
} from './web-link.js';
import { Profile } from './web-profile.js';

export {
  isNavigationClick,
  type Navigate,
  type NavigationIcon,
  type NavigationIconState,
  type NavigationLinkProps,
  type NavigationLinkRenderer,
} from './web-link.js';
export { SectionPicker, type SectionPickerProps } from './web-section-picker.js';

const ICON_SIZE = 24;
const CHILD_ICON_SIZE = 20;

function subscribeWidth(change: () => void): () => void {
  window.addEventListener('resize', change);
  return () => {
    window.removeEventListener('resize', change);
  };
}
function viewportWidth(): number {
  return window.innerWidth;
}
function serverWidth(): number {
  return 0;
}

export interface AppNavigationProps extends CollapseProps, NavigationSlots {
  readonly closeLabel: string;
  readonly items: readonly NavigationItem<NavigationIcon>[];
  readonly profile?: NavigationProfile;
  readonly pathname: string;
  readonly onNavigate?: Navigate;
  readonly renderLink?: NavigationLinkRenderer;
  readonly label: string;
  readonly collapseLabel: string;
  readonly expandLabel: string;
  readonly width?: number;
  readonly className?: string;
}

type NavigationLayout = ReturnType<typeof getNavigationLayout>;
type VisibleItem = NavigationItem<NavigationIcon> | NavigationSubItem<NavigationIcon>;

interface NavigationModel {
  readonly props: AppNavigationProps;
  readonly layout: NavigationLayout;
  readonly collapsed: boolean;
  readonly state: NavigationStateHandle<NavigationIcon>;
  readonly id: string;
  readonly visible: readonly VisibleItem[];
  readonly roving: ReturnType<typeof useRovingMenu>;
}

function useBarHeightReport(
  navigationRef: RefObject<HTMLElement | null>,
  layout: NavigationLayout,
  onBarHeightChange: ((height: number) => void) | undefined,
): void {
  useLayoutEffect(() => {
    const navigation = navigationRef.current;
    if (layout !== 'bar' || navigation === null || onBarHeightChange === undefined) return;
    const measure = (): void => {
      onBarHeightChange(navigation.getBoundingClientRect().height);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(navigation);
    return () => {
      observer.disconnect();
    };
  }, [navigationRef, layout, onBarHeightChange]);
}

function visibleItems(
  items: readonly NavigationItem<NavigationIcon>[],
  expanded: boolean,
  openIds: readonly string[],
): VisibleItem[] {
  return items.flatMap((item) =>
    expanded && openIds.includes(item.id) ? [item, ...(item.children ?? [])] : [item],
  );
}

function useNavigationModel(props: AppNavigationProps): NavigationModel {
  const { items, pathname, profile } = props;
  validateNavigation(items, profile);
  useAriaHiddenInert();
  const measuredWidth = useSyncExternalStore(subscribeWidth, viewportWidth, serverWidth);
  const layout = getNavigationLayout(props.width ?? measuredWidth);
  const state = useNavigationState(items, pathname, props);
  const collapsed = layout === 'rail' && state.collapsed;
  const id = useId();
  const visible = visibleItems(items, layout === 'rail' && !collapsed, state.openIds);
  const activeId = visible.some((item) => item.id === state.active.subItem?.id)
    ? state.active.subItem?.id
    : state.active.item?.id;
  const roving = useRovingMenu({
    active: true,
    options: [
      ...visible.map((item) => ({ selected: item.id === activeId })),
      ...(profile === undefined ? [] : [{}]),
    ],
  });
  return { props, layout, collapsed, state, id, visible, roving };
}

function rendersSomething(node: ReactNode): boolean {
  return node !== null && node !== undefined && typeof node !== 'boolean';
}

function Slots({ model }: { readonly model: NavigationModel }): ReactNode {
  const slotState = { layout: model.layout, collapsed: model.collapsed };
  const brand = model.props.renderBrand?.(slotState);
  const accessory = model.props.renderAccessory?.(slotState);
  const hasBrand = rendersSomething(brand);
  const hasAccessory = rendersSomething(accessory);
  if (!hasBrand && !hasAccessory) return null;
  return (
    <div className="bk-navigation-slots">
      {!hasBrand ? null : <div className="bk-navigation-brand">{brand}</div>}
      {!hasAccessory ? null : <div className="bk-navigation-accessory">{accessory}</div>}
    </div>
  );
}

function CollapseToggle({ model }: { readonly model: NavigationModel }): ReactNode {
  const { collapsed, props } = model;
  return (
    <button
      className="bk-navigation-toggle"
      type="button"
      aria-label={collapsed ? props.expandLabel : props.collapseLabel}
      aria-expanded={!collapsed}
      onClick={() => {
        model.state.setCollapsed(!collapsed);
      }}
    >
      <span aria-hidden="true">{collapsed ? '›' : '‹'}</span>
    </button>
  );
}

function SectionChildren({
  model,
  item,
  groupId,
  open,
}: {
  readonly model: NavigationModel;
  readonly item: NavigationItem<NavigationIcon>;
  readonly groupId: string;
  readonly open: boolean;
}): ReactNode {
  const { state, roving, visible, props } = model;
  return (
    <div
      className="bk-navigation-children"
      id={groupId}
      role="group"
      aria-label={item.label}
      hidden={!open}
    >
      {(item.children ?? []).map((child) => {
        const selected = state.active.subItem?.id === child.id;
        return (
          <Link
            {...roving.itemProps(visible.indexOf(child))}
            renderLink={props.renderLink}
            key={child.id}
            className="bk-navigation-item"
            href={child.href}
            aria-current={selected ? 'page' : undefined}
            data-active={selected ? 'page' : undefined}
            onClick={(event) => {
              follow(child.href, event, props.onNavigate);
            }}
          >
            {child.icon === undefined ? null : (
              <span className="bk-navigation-icon" aria-hidden="true">
                {child.icon({ active: selected, size: CHILD_ICON_SIZE })}
              </span>
            )}
            {child.label}
          </Link>
        );
      })}
    </div>
  );
}

function activeMarker(active: boolean, ancestor: boolean): 'ancestor' | 'page' | undefined {
  if (!active) return undefined;
  return ancestor ? 'ancestor' : 'page';
}

function sectionHref(item: NavigationItem, cycle: boolean, pathname: string): string {
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
  const { layout, collapsed, state, props } = model;
  const active = state.active.item?.id === item.id;
  const rail = layout === 'rail';
  const ancestor = active && rail && !collapsed && state.active.subItem !== null;
  const group = rail && (item.children ?? []).length > 0;
  const open = !collapsed && state.openIds.includes(item.id);
  const groupId = `${model.id}-${String(index)}`;
  const content = (
    <>
      <span className="bk-navigation-icon" aria-hidden="true">
        {item.icon({ active, size: ICON_SIZE })}
      </span>
      <span className="bk-navigation-label">{item.label}</span>
    </>
  );
  const common = {
    ...model.roving.itemProps(model.visible.indexOf(item)),
    className: 'bk-navigation-item',
    'aria-label': item.label,
    'data-active': activeMarker(active, ancestor),
    title: collapsed ? item.label : undefined,
  };
  const href = sectionHref(item, layout === 'bar' && active, props.pathname);
  return (
    <div className="bk-navigation-section" data-open={open}>
      {group ? (
        <button
          {...common}
          type="button"
          aria-expanded={open}
          aria-controls={groupId}
          onClick={() => {
            state.toggleGroup(item.id);
          }}
        >
          {content}
        </button>
      ) : (
        <Link
          {...common}
          renderLink={props.renderLink}
          href={href}
          aria-current={active ? 'page' : undefined}
          onClick={(event) => {
            follow(href, event, props.onNavigate);
          }}
        >
          {content}
        </Link>
      )}
      {group ? <SectionChildren model={model} item={item} groupId={groupId} open={open} /> : null}
    </div>
  );
}

function barGridStyle(model: NavigationModel): CSSProperties | undefined {
  if (model.layout !== 'bar') return undefined;
  const { items, profile } = model.props;
  const columns = Math.max(1, items.length + (profile === undefined ? 0 : 1));
  return {
    gridTemplateColumns: `repeat(${String(columns)}, minmax(${String(NAVIGATION_DIMENSIONS.target)}px, 1fr))`,
  };
}

function motionState(resolved: boolean, reducedMotion: boolean): string {
  if (!resolved) return 'unresolved';
  return reducedMotion ? 'reduced' : 'standard';
}

export function AppNavigation(props: AppNavigationProps): ReactNode {
  const model = useNavigationModel(props);
  const { layout, collapsed } = model;
  const { reducedMotion, resolved } = useReducedMotionPreference();
  const navigationRef = useRef<HTMLElement>(null);
  useBarHeightReport(navigationRef, layout, props.onBarHeightChange);
  const profile = props.profile;
  return (
    <nav
      ref={navigationRef}
      style={barGridStyle(model)}
      aria-label={props.label}
      className={`bk-navigation ${props.className ?? ''}`}
      data-testid="primary-navigation"
      data-layout={layout}
      data-collapsed={collapsed}
      data-motion={motionState(resolved, reducedMotion)}
    >
      <Slots model={model} />
      {layout === 'rail' ? <CollapseToggle model={model} /> : null}
      <div className="bk-navigation-items">
        {props.items.map((item, index) => (
          <NavigationSection key={item.id} model={model} item={item} index={index} />
        ))}
      </div>
      {profile === undefined ? null : (
        <Profile
          closeLabel={props.closeLabel}
          profile={profile}
          pathname={props.pathname}
          collapsed={collapsed}
          itemProps={model.roving.itemProps(model.visible.length)}
          onNavigate={props.onNavigate}
          renderLink={props.renderLink}
        />
      )}
    </nav>
  );
}

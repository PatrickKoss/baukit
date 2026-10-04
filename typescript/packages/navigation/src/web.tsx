import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ComponentPropsWithRef,
  type MouseEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import {
  announce,
  useAriaHiddenInert,
  useReducedMotionPreference,
  useRovingMenu,
} from '@baukit/a11y-core/web';
import {
  getNavigationLayout,
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
export type NavigationLinkProps = ComponentPropsWithRef<'a'> & { readonly href: string };
export type NavigationLinkRenderer = (props: NavigationLinkProps) => ReactNode;
export type Navigate = (href: string, event: MouseEvent<HTMLAnchorElement>) => void;

export function isNavigationClick(event: MouseEvent<HTMLAnchorElement>): boolean {
  return (
    !event.defaultPrevented &&
    event.button === 0 &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.shiftKey &&
    !event.altKey
  );
}

function follow(href: string, event: MouseEvent<HTMLAnchorElement>, onNavigate?: Navigate) {
  if (!isNavigationClick(event) || onNavigate === undefined) return;
  event.preventDefault();
  onNavigate(href, event);
}

function subscribeWidth(change: () => void) {
  window.addEventListener('resize', change);
  return () => {
    window.removeEventListener('resize', change);
  };
}
function viewportWidth() {
  return window.innerWidth;
}
function serverWidth() {
  return 0;
}
function Link({
  renderLink,
  ...props
}: NavigationLinkProps & { readonly renderLink: NavigationLinkRenderer | undefined }) {
  return renderLink === undefined ? <a {...props} /> : renderLink(props);
}

interface MenuProps {
  readonly entries: readonly NavigationProfileMenuEntry[];
  readonly id: string;
  readonly label: string;
  readonly triggerRef: RefObject<HTMLButtonElement | null>;
  readonly onClose: () => void;
  readonly onNavigate: Navigate | undefined;
  readonly renderLink: NavigationLinkRenderer | undefined;
  readonly pathname: string;
  readonly maxHeight?: number | undefined;
}

function Menu({
  entries,
  id,
  label,
  triggerRef,
  onClose,
  onNavigate,
  renderLink,
  pathname,
  maxHeight,
}: MenuProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const roving = useRovingMenu({
    active: true,
    options: entries.map((entry) => ({ disabled: entry.disabled })),
  });
  useEffect(() => {
    const menu = containerRef.current;
    const first = menu?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])');
    (first ?? menu)?.focus();
  }, []);
  useEffect(() => {
    function outside(event: PointerEvent) {
      if (
        !(event.target instanceof Node) ||
        containerRef.current?.contains(event.target) ||
        triggerRef.current?.contains(event.target)
      )
        return;
      onClose();
    }
    document.addEventListener('pointerdown', outside);
    return () => {
      document.removeEventListener('pointerdown', outside);
    };
  }, [onClose, triggerRef]);
  function closeAndRestore() {
    onClose();
    triggerRef.current?.focus();
  }
  return (
    <div
      className="bk-navigation-menu"
      style={maxHeight === undefined ? undefined : { maxHeight }}
      id={id}
      role="menu"
      aria-label={label}
      tabIndex={-1}
      ref={containerRef}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          closeAndRestore();
        }
        if (event.key === 'Tab') {
          // Move focus to the trigger before removing the focused menu item. The browser continues Tab normally.
          triggerRef.current?.focus();
          onClose();
        }
      }}
    >
      {entries.map((entry, index) => {
        const props = {
          ...roving.itemProps(index),
          className: 'bk-navigation-item',
          role: 'menuitem',
          'aria-disabled': entry.disabled === true ? true : undefined,
          'data-active':
            entry.href !== undefined && navigationMatches({ ...entry, href: entry.href }, pathname)
              ? 'page'
              : undefined,
        } as const;
        return entry.href !== undefined ? (
          <Link
            {...props}
            renderLink={renderLink}
            href={entry.href}
            key={entry.id}
            aria-current={entry.href === pathname ? 'page' : undefined}
            onClick={(event) => {
              if (entry.disabled === true) {
                event.preventDefault();
                return;
              }
              if (!isNavigationClick(event)) return;
              closeAndRestore();
              follow(entry.href, event, onNavigate);
            }}
            onKeyDown={(event) => {
              props.onKeyDown(event);
              if (event.key === ' ' && entry.disabled !== true) {
                event.preventDefault();
                event.currentTarget.click();
              }
            }}
          >
            {entry.label}
          </Link>
        ) : (
          <button
            {...props}
            type="button"
            disabled={entry.disabled}
            key={entry.id}
            onClick={() => {
              closeAndRestore();
              entry.onSelect();
            }}
          >
            {entry.label}
          </button>
        );
      })}
    </div>
  );
}

function Avatar({ profile }: { readonly profile: NavigationProfile }) {
  const [failedUrl, setFailedUrl] = useState<string>();
  return (
    <span className="bk-navigation-avatar" aria-hidden="true">
      {profile.imageUrl !== undefined && failedUrl !== profile.imageUrl ? (
        <img
          src={profile.imageUrl}
          alt=""
          onError={() => {
            setFailedUrl(profile.imageUrl);
          }}
        />
      ) : (
        profile.initials
      )}
    </span>
  );
}

interface ProfileProps {
  readonly profile: NavigationProfile;
  readonly pathname: string;
  readonly collapsed: boolean;
  readonly itemProps: ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;
  readonly onNavigate: Navigate | undefined;
  readonly renderLink: NavigationLinkRenderer | undefined;
}
function Profile({
  profile,
  pathname,
  collapsed,
  itemProps,
  onNavigate,
  renderLink,
}: ProfileProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const id = useId();
  const active =
    profile.href !== undefined
      ? navigationMatches({ id: 'profile', label: profile.label, href: profile.href }, pathname)
      : profile.menu.some(
          (entry) =>
            entry.href !== undefined && navigationMatches({ ...entry, href: entry.href }, pathname),
        );
  const content = (
    <>
      <Avatar profile={profile} />
      <span className="bk-navigation-label">{profile.label}</span>
    </>
  );
  const common = {
    ...itemProps,
    className: 'bk-navigation-item',
    'aria-label': profile.label,
    'data-active': active ? 'page' : undefined,
    title: collapsed ? profile.label : undefined,
  };
  return (
    <div className="bk-navigation-profile">
      {profile.href !== undefined ? (
        <Link
          {...common}
          renderLink={renderLink}
          href={profile.href}
          aria-current={active ? 'page' : undefined}
          onClick={(event) => {
            follow(profile.href, event, onNavigate);
          }}
        >
          {content}
        </Link>
      ) : (
        <button
          {...common}
          type="button"
          ref={(node) => {
            triggerRef.current = node;
            itemProps.ref(node);
          }}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          onClick={() => {
            setOpen((value) => !value);
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault();
              setOpen(true);
            } else itemProps.onKeyDown(event);
          }}
        >
          {content}
        </button>
      )}
      {open && profile.menu !== undefined ? (
        <Menu
          entries={profile.menu}
          id={id}
          label={profile.label}
          triggerRef={triggerRef}
          onClose={() => {
            setOpen(false);
          }}
          pathname={pathname}
          onNavigate={onNavigate}
          renderLink={renderLink}
        />
      ) : null}
    </div>
  );
}

export interface AppNavigationProps extends CollapseProps {
  readonly items: readonly NavigationItem<NavigationIcon>[];
  readonly profile?: NavigationProfile;
  readonly pathname: string;
  readonly onNavigate?: Navigate;
  readonly renderLink?: NavigationLinkRenderer;
  readonly label?: string;
  readonly collapseLabel?: string;
  readonly expandLabel?: string;
  readonly width?: number;
  readonly className?: string;
}

export function AppNavigation(props: AppNavigationProps) {
  const {
    items,
    pathname,
    profile,
    onNavigate,
    renderLink,
    label = 'Primary',
    collapseLabel = 'Collapse navigation',
    expandLabel = 'Expand navigation',
  } = props;
  validateNavigation(items, profile);
  useAriaHiddenInert();
  const measuredWidth = useSyncExternalStore(subscribeWidth, viewportWidth, serverWidth);
  const layout = getNavigationLayout(props.width ?? measuredWidth);
  const state = useNavigationState(items, pathname, props);
  const collapsed = layout === 'rail' && state.collapsed;
  const { reducedMotion, resolved } = useReducedMotionPreference();
  const id = useId();
  const visible = items.flatMap((item) =>
    layout === 'rail' && !collapsed && state.openIds.includes(item.id)
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
    <nav
      aria-label={label}
      className={`bk-navigation ${props.className ?? ''}`}
      data-testid="primary-navigation"
      data-layout={layout}
      data-collapsed={collapsed}
      data-motion={!resolved ? 'unresolved' : reducedMotion ? 'reduced' : 'standard'}
    >
      {layout === 'rail' ? (
        <button
          className="bk-navigation-toggle"
          type="button"
          aria-label={collapsed ? expandLabel : collapseLabel}
          aria-expanded={!collapsed}
          onClick={() => {
            state.setCollapsed(!collapsed);
          }}
        >
          <span aria-hidden="true">{collapsed ? '›' : '‹'}</span>
        </button>
      ) : null}
      <div className="bk-navigation-items">
        {items.map((item, index) => {
          const active = state.active.item?.id === item.id;
          const ancestor =
            active && layout === 'rail' && !collapsed && state.active.subItem !== null;
          const children = item.children ?? [];
          const open = !collapsed && state.openIds.includes(item.id);
          const groupId = `${id}-${String(index)}`;
          const itemProps = roving.itemProps(visible.indexOf(item));
          const content = (
            <>
              <span className="bk-navigation-icon" aria-hidden="true">
                {item.icon({ active, size: 24 })}
              </span>
              <span className="bk-navigation-label">{item.label}</span>
            </>
          );
          const common = {
            ...itemProps,
            className: 'bk-navigation-item',
            'aria-label': item.label,
            'data-active': active ? (ancestor ? 'ancestor' : 'page') : undefined,
            title: collapsed ? item.label : undefined,
          };
          const href = layout === 'bar' && active ? nextSectionHref(item, pathname) : item.href;
          return (
            <div className="bk-navigation-section" key={item.id} data-open={open}>
              {layout === 'rail' && children.length > 0 ? (
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
                  renderLink={renderLink}
                  href={href}
                  aria-current={active ? 'page' : undefined}
                  onClick={(event) => {
                    follow(href, event, onNavigate);
                  }}
                >
                  {content}
                </Link>
              )}
              {layout === 'rail' && children.length > 0 ? (
                <div
                  className="bk-navigation-children"
                  id={groupId}
                  role="group"
                  aria-label={item.label}
                  hidden={!open}
                >
                  {children.map((child) => {
                    const selected = state.active.subItem?.id === child.id;
                    return (
                      <Link
                        {...roving.itemProps(visible.indexOf(child))}
                        renderLink={renderLink}
                        key={child.id}
                        className="bk-navigation-item"
                        href={child.href}
                        aria-current={selected ? 'page' : undefined}
                        data-active={selected ? 'page' : undefined}
                        onClick={(event) => {
                          follow(child.href, event, onNavigate);
                        }}
                      >
                        {child.icon === undefined ? null : (
                          <span className="bk-navigation-icon" aria-hidden="true">
                            {child.icon({ active: selected, size: 20 })}
                          </span>
                        )}
                        {child.label}
                      </Link>
                    );
                  })}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      {profile === undefined ? null : (
        <Profile
          profile={profile}
          pathname={pathname}
          collapsed={collapsed}
          itemProps={roving.itemProps(visible.length)}
          onNavigate={onNavigate}
          renderLink={renderLink}
        />
      )}
    </nav>
  );
}

const SECTION_MENU_GAP = 8;

function useSectionMenuHeight(open: boolean, triggerRef: RefObject<HTMLButtonElement | null>) {
  const [height, setHeight] = useState<number>();
  const measuredHeight = useRef<number | undefined>(undefined);
  useLayoutEffect(() => {
    const trigger = triggerRef.current;
    if (!open || trigger === null) return;
    function measure() {
      const bar = document.querySelector<HTMLElement>(".bk-navigation[data-layout='bar']");
      if (trigger === null) return;
      const viewportBottom =
        (window.visualViewport?.height ?? window.innerHeight) +
        (window.visualViewport?.offsetTop ?? 0);
      const bottom = Math.min(viewportBottom, bar?.getBoundingClientRect().top ?? viewportBottom);
      const nextHeight = Math.max(
        0,
        bottom - trigger.getBoundingClientRect().bottom - SECTION_MENU_GAP * 2,
      );
      if (measuredHeight.current === nextHeight) return;
      measuredHeight.current = nextHeight;
      setHeight(nextHeight);
    }
    measure();
    const observer = 'ResizeObserver' in window ? new ResizeObserver(measure) : undefined;
    for (
      let element: HTMLElement | null = trigger;
      element !== null;
      element = element.parentElement
    ) {
      observer?.observe(element);
    }
    const navigation = document.querySelector('.bk-navigation');
    if (navigation !== null) observer?.observe(navigation);
    window.addEventListener('resize', measure);
    window.addEventListener('scroll', measure, true);
    window.visualViewport?.addEventListener('resize', measure);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', measure);
      window.removeEventListener('scroll', measure, true);
      window.visualViewport?.removeEventListener('resize', measure);
    };
  }, [open, triggerRef]);
  return height;
}

export interface SectionPickerProps {
  readonly item: NavigationItem<NavigationIcon>;
  readonly pathname: string;
  readonly onNavigate?: Navigate;
  readonly renderLink?: NavigationLinkRenderer;
}
export function SectionPicker({ item, pathname, onNavigate, renderLink }: SectionPickerProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const maxHeight = useSectionMenuHeight(open, triggerRef);
  const id = useId();
  const active = resolveActiveNavigation([item], pathname).subItem ?? item.children?.[0];
  const entries = item.children ?? [];
  if (entries.length === 0) return null;
  const label = `${item.label}, ${active?.label ?? ''}`;
  return (
    <nav aria-label={item.label} className="bk-navigation-picker">
      <button
        className="bk-navigation-picker-trigger"
        type="button"
        ref={triggerRef}
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-controls={open ? id : undefined}
        onClick={() => {
          setOpen((value) => !value);
        }}
      >
        <span>
          <span className="bk-navigation-picker-section">{item.label}</span>
          <span>{active?.label}</span>
        </span>
        <span aria-hidden="true">⌄</span>
      </button>
      {open ? (
        <Menu
          entries={entries}
          maxHeight={maxHeight}
          id={id}
          label={item.label}
          triggerRef={triggerRef}
          onClose={() => {
            setOpen(false);
          }}
          pathname={pathname}
          onNavigate={
            onNavigate === undefined
              ? undefined
              : (href, event) => {
                  announce(entries.find((entry) => entry.href === href)?.label ?? item.label);
                  onNavigate(href, event);
                }
          }
          renderLink={renderLink}
        />
      ) : null}
    </nav>
  );
}

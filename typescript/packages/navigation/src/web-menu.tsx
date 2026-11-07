import { useEffect, useRef, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { flushSync } from 'react-dom';
import { useRovingMenu } from '@baukit/a11y-core/web';
import { resolveActiveMenuEntry, type NavigationProfileMenuEntry } from './index.js';
import {
  follow,
  isNavigationClick,
  Link,
  type Navigate,
  type NavigationLinkRenderer,
} from './web-link.js';

export interface MenuProps {
  readonly replace?: boolean;
  readonly entries: readonly NavigationProfileMenuEntry[];
  readonly id: string;
  readonly label: string;
  readonly closeLabel: string;
  readonly triggerRef: RefObject<HTMLButtonElement | null>;
  readonly onClose: () => void;
  readonly onNavigate: Navigate | undefined;
  readonly renderLink: NavigationLinkRenderer | undefined;
  readonly pathname: string;
  readonly maxHeight?: number | undefined;
  readonly subtitle?: string | undefined;
}

function useFocusFirstItem(menuRef: RefObject<HTMLDivElement | null>): void {
  useEffect(() => {
    const menu = menuRef.current;
    const first = menu?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])');
    (first ?? menu)?.focus();
  }, [menuRef]);
}

function useCloseOnOutsidePress(
  containerRef: RefObject<HTMLDivElement | null>,
  triggerRef: RefObject<HTMLButtonElement | null>,
  onClose: () => void,
): void {
  useEffect(() => {
    function outside(event: PointerEvent): void {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (containerRef.current?.contains(target) === true) return;
      if (triggerRef.current?.contains(target) === true) return;
      onClose();
    }
    document.addEventListener('pointerdown', outside);
    return () => {
      document.removeEventListener('pointerdown', outside);
    };
  }, [containerRef, onClose, triggerRef]);
}

type ItemProps = ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;

interface MenuEntryProps {
  readonly entry: NavigationProfileMenuEntry;
  readonly active: boolean;
  readonly itemProps: ItemProps;
  readonly replace: boolean | undefined;
  readonly renderLink: NavigationLinkRenderer | undefined;
  readonly onNavigate: Navigate | undefined;
  readonly onCloseAndRestore: () => void;
}

function MenuEntry({
  entry,
  active,
  itemProps,
  replace,
  renderLink,
  onNavigate,
  onCloseAndRestore,
}: MenuEntryProps): ReactNode {
  const props = {
    ...itemProps,
    className: 'bk-navigation-item',
    role: 'menuitem',
    'aria-disabled': entry.disabled === true ? true : undefined,
    'data-active': active ? 'page' : undefined,
    'data-tone': entry.tone,
  } as const;
  const href = entry.href;
  if (href === undefined) {
    return (
      <button
        {...props}
        type="button"
        disabled={entry.disabled}
        onClick={() => {
          onCloseAndRestore();
          entry.onSelect();
        }}
      >
        {entry.label}
      </button>
    );
  }
  return (
    <Link
      {...props}
      {...(replace === undefined ? {} : { replace })}
      renderLink={renderLink}
      href={href}
      aria-current={active ? 'page' : undefined}
      onClick={(event) => {
        if (entry.disabled === true) {
          event.preventDefault();
          return;
        }
        if (!isNavigationClick(event)) return;
        onCloseAndRestore();
        follow(href, event, onNavigate);
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
  );
}

function useMenuKeyHandler(
  triggerRef: RefObject<HTMLButtonElement | null>,
  onClose: () => void,
  closeAndRestore: () => void,
): (event: KeyboardEvent<HTMLDivElement>) => void {
  return (event) => {
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
  };
}

export function Menu({
  replace,
  entries,
  id,
  label,
  closeLabel,
  triggerRef,
  onClose,
  onNavigate,
  renderLink,
  pathname,
  maxHeight,
  subtitle,
}: MenuProps): ReactNode {
  const activeEntry = resolveActiveMenuEntry(entries, pathname);
  const containerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const roving = useRovingMenu({
    active: true,
    options: entries.map((entry) => ({ disabled: entry.disabled })),
  });
  useFocusFirstItem(menuRef);
  useCloseOnOutsidePress(containerRef, triggerRef, onClose);
  function closeAndRestore(): void {
    flushSync(onClose);
    triggerRef.current?.focus();
  }
  const onKeyDown = useMenuKeyHandler(triggerRef, onClose, closeAndRestore);
  return (
    // The wrapper only handles keys that bubble up from the menu items and the close button.
    <div
      className="bk-navigation-menu"
      role="presentation"
      style={maxHeight === undefined ? undefined : { maxHeight }}
      ref={containerRef}
      onKeyDown={onKeyDown}
    >
      <div
        className="bk-navigation-menu-content"
        id={id}
        role="menu"
        aria-label={subtitle === undefined ? label : `${label}, ${subtitle}`}
        tabIndex={-1}
        ref={menuRef}
      >
        {subtitle === undefined ? null : (
          <div className="bk-navigation-menu-profile" aria-hidden="true">
            <span className="bk-navigation-label">{label}</span>
            <span className="bk-navigation-subtitle">{subtitle}</span>
          </div>
        )}
        <div className="bk-navigation-menu-items">
          {entries.map((entry, index) => (
            <MenuEntry
              key={entry.id}
              entry={entry}
              active={entry === activeEntry}
              itemProps={roving.itemProps(index)}
              replace={replace}
              renderLink={renderLink}
              onNavigate={onNavigate}
              onCloseAndRestore={closeAndRestore}
            />
          ))}
        </div>
      </div>
      <button type="button" className="bk-navigation-menu-close" onClick={closeAndRestore}>
        {closeLabel}
      </button>
    </div>
  );
}

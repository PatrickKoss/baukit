import { useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { announce } from '@baukit/a11y-core/web';
import { resolveActiveNavigation, type NavigationItem } from './index.js';
import type { Navigate, NavigationIcon, NavigationLinkRenderer } from './web-link.js';
import { Menu } from './web-menu.js';

const SECTION_MENU_GAP = 8;

function useSectionMenuHeight(
  open: boolean,
  triggerRef: RefObject<HTMLButtonElement | null>,
): number | undefined {
  const [height, setHeight] = useState<number>();
  const measuredHeight = useRef<number | undefined>(undefined);
  useLayoutEffect(() => {
    const trigger = triggerRef.current;
    if (!open || trigger === null) return;
    function measure(): void {
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
  readonly replace?: boolean;
  readonly closeLabel: string;
  readonly item: NavigationItem<NavigationIcon>;
  readonly pathname: string;
  readonly onNavigate?: Navigate;
  readonly renderLink?: NavigationLinkRenderer;
}
export function SectionPicker({
  replace = false,
  item,
  pathname,
  onNavigate,
  renderLink,
  closeLabel,
}: SectionPickerProps): ReactNode {
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
          {...(replace ? { replace } : {})}
          closeLabel={closeLabel}
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
                  if (replace) onNavigate(href, event, { replace: true });
                  else onNavigate(href, event);
                }
          }
          renderLink={renderLink}
        />
      ) : null}
    </nav>
  );
}

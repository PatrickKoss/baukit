import type { ComponentPropsWithRef, MouseEvent, ReactNode } from 'react';

export interface NavigationIconState {
  readonly active: boolean;
  readonly size: number;
}
export type NavigationIcon = (state: NavigationIconState) => ReactNode;
export type NavigationLinkProps = ComponentPropsWithRef<'a'> & {
  readonly href: string;
  readonly replace?: boolean;
};
export type NavigationLinkRenderer = (props: NavigationLinkProps) => ReactNode;
export type Navigate = (
  href: string,
  event: MouseEvent<HTMLAnchorElement>,
  options?: { readonly replace: boolean },
) => void;

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

export function follow(
  href: string,
  event: MouseEvent<HTMLAnchorElement>,
  onNavigate?: Navigate,
): void {
  if (!isNavigationClick(event) || onNavigate === undefined) return;
  event.preventDefault();
  onNavigate(href, event);
}

export function Link({
  renderLink,
  replace,
  children,
  ...props
}: NavigationLinkProps & { readonly renderLink: NavigationLinkRenderer | undefined }): ReactNode {
  if (renderLink !== undefined) {
    const linkProps = { ...props, children };
    return renderLink(replace === undefined ? linkProps : { ...linkProps, replace });
  }
  return <a {...props}>{children}</a>;
}

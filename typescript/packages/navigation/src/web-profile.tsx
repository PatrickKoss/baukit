import {
  useId,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type RefObject,
  type SetStateAction,
} from 'react';
import type { useRovingMenu } from '@baukit/a11y-core/web';
import {
  getNavigationProfileLabel,
  NAVIGATION_DIMENSIONS,
  navigationMatches,
  resolveActiveMenuEntry,
  type NavigationProfile,
} from './index.js';
import { follow, Link, type Navigate, type NavigationLinkRenderer } from './web-link.js';
import { Menu } from './web-menu.js';

export type RovingItemProps = ReturnType<ReturnType<typeof useRovingMenu>['itemProps']>;

function Avatar({
  profile,
  active,
}: {
  readonly profile: NavigationProfile;
  readonly active: boolean;
}): ReactNode {
  const [failedUrl, setFailedUrl] = useState<string>();
  if (profile.renderAvatar !== undefined) {
    return (
      <span className="bk-navigation-avatar bk-navigation-avatar-custom" aria-hidden="true">
        {profile.renderAvatar({ active, size: NAVIGATION_DIMENSIONS.avatar })}
      </span>
    );
  }
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

function ProfileContent({
  profile,
  active,
}: {
  readonly profile: NavigationProfile;
  readonly active: boolean;
}): ReactNode {
  return (
    <>
      <Avatar profile={profile} active={active} />
      <span className="bk-navigation-profile-copy">
        <span className="bk-navigation-label">{profile.label}</span>
        {profile.subtitle === undefined ? null : (
          <span className="bk-navigation-subtitle">{profile.subtitle}</span>
        )}
      </span>
    </>
  );
}

function profileIsActive(profile: NavigationProfile, pathname: string): boolean {
  return profile.href !== undefined
    ? navigationMatches({ id: 'profile', label: profile.label, href: profile.href }, pathname)
    : resolveActiveMenuEntry(profile.menu, pathname) !== null;
}

interface MenuTriggerProps {
  readonly common: object;
  readonly itemProps: RovingItemProps;
  readonly triggerRef: RefObject<HTMLButtonElement | null>;
  readonly open: boolean;
  readonly menuId: string;
  readonly setOpen: Dispatch<SetStateAction<boolean>>;
  readonly children: ReactNode;
}

function MenuTrigger({
  common,
  itemProps,
  triggerRef,
  open,
  menuId,
  setOpen,
  children,
}: MenuTriggerProps): ReactNode {
  return (
    <button
      {...common}
      type="button"
      ref={(node) => {
        triggerRef.current = node;
        itemProps.ref(node);
      }}
      aria-haspopup="menu"
      aria-expanded={open}
      aria-controls={open ? menuId : undefined}
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
      {children}
    </button>
  );
}

export interface ProfileProps {
  readonly closeLabel: string;
  readonly profile: NavigationProfile;
  readonly pathname: string;
  readonly collapsed: boolean;
  readonly itemProps: RovingItemProps;
  readonly onNavigate: Navigate | undefined;
  readonly renderLink: NavigationLinkRenderer | undefined;
}
export function Profile({
  closeLabel,
  profile,
  pathname,
  collapsed,
  itemProps,
  onNavigate,
  renderLink,
}: ProfileProps): ReactNode {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const id = useId();
  const active = profileIsActive(profile, pathname);
  const profileLabel = getNavigationProfileLabel(profile);
  const common = {
    ...itemProps,
    className: 'bk-navigation-item',
    'aria-label': profileLabel,
    'data-active': active ? 'page' : undefined,
    title: collapsed ? profileLabel : undefined,
  };
  const content = <ProfileContent profile={profile} active={active} />;
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
        <MenuTrigger
          common={common}
          itemProps={itemProps}
          triggerRef={triggerRef}
          open={open}
          menuId={id}
          setOpen={setOpen}
        >
          {content}
        </MenuTrigger>
      )}
      {open && profile.menu !== undefined ? (
        <Menu
          closeLabel={closeLabel}
          entries={profile.menu}
          id={id}
          label={profile.label}
          subtitle={profile.subtitle}
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

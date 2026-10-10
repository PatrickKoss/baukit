{% raw %}
import { useEffect, useRef, useState } from "react";
import {
  AppNavigation,
  SectionPicker,
  type NavigationIcon,
} from "@baukit/navigation/web";
import type { NavigationItem, NavigationProfile } from "@baukit/navigation";
import { englishCatalog } from "./localization/en";
import { germanCatalog } from "./localization/de";
import { createRouteFocusController } from "@baukit/a11y-core/web";

const homeIcon: NavigationIcon = ({ size }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
    <path
      d="m3 10 9-7 9 7v11H3zM9 21v-8h6v8"
      stroke="currentColor"
      strokeWidth="2"
    />
  </svg>
);
const itemsIcon: NavigationIcon = ({ size }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
    <path d="M4 5h16M4 12h16M4 19h16" stroke="currentColor" strokeWidth="2" />
  </svg>
);
function navigationItems(
  copy: typeof englishCatalog.navigation | typeof germanCatalog.navigation,
) {
  const section: NavigationItem<NavigationIcon> = {
    id: "workspace",
    label: copy.workspace,
    href: "/#items-title",
    matches: (path) => path === "/#items-title" || path === "/#privacy-title",
    icon: itemsIcon,
    children: [
      {
        id: "items",
        label: copy.items,
        href: "/#items-title",
        matches: (path) => path === "/#items-title",
      },
      {
        id: "privacy",
        label: copy.privacy,
        href: "/#privacy-title",
        matches: (path) => path === "/#privacy-title",
      },
    ],
  };
  const items: readonly NavigationItem<NavigationIcon>[] = [
    {
      id: "home",
      label: copy.home,
      href: "/",
      icon: homeIcon,
      matches: (path) => path === "/",
    },
    section,
  ];
  return { section, items };
}

export function NavigationShell({
  profile,
  onLocationChange,
}: {
  readonly profile?: NavigationProfile;
  readonly onLocationChange?: () => void;
}) {
  const copy =
    navigator.language.toLowerCase().split("-")[0] === "de"
      ? germanCatalog.navigation
      : englishCatalog.navigation;
  const { section, items } = navigationItems(copy);
  const [pathname, setPathname] = useState(
    window.location.pathname + window.location.hash,
  );
  const locationChanged = useRef(onLocationChange);
  useEffect(() => {
    locationChanged.current = onLocationChange;
  });
  const routeFocus = useRef<ReturnType<
    typeof createRouteFocusController
  > | null>(null);
  useEffect(() => {
    const controller = createRouteFocusController();
    routeFocus.current = controller;
    const update = () => {
      setPathname(window.location.pathname + window.location.hash);
      locationChanged.current?.();
    };
    window.addEventListener("popstate", update);
    window.addEventListener("hashchange", update);
    return () => {
      controller.dispose();
      window.removeEventListener("popstate", update);
      window.removeEventListener("hashchange", update);
    };
  }, []);
  useEffect(
    () =>
      routeFocus.current?.enterRoute(() => {
        const hash = pathname.split("#")[1];
        const heading =
          hash === undefined
            ? document.querySelector<HTMLElement>("h1")
            : document.getElementById(hash);
        if (heading !== null) heading.tabIndex = -1;
        return heading;
      }),
    [pathname],
  );
  function navigate(href: string) {
    window.history.pushState(null, "", href);
    setPathname(window.location.pathname + window.location.hash);
    onLocationChange?.();
  }
  return (
    <>
      <AppNavigation
        closeLabel={copy.close}
        label={copy.primary}
        collapseLabel={copy.collapse}
        expandLabel={copy.expand}
        items={items}
        pathname={pathname}
        onNavigate={navigate}
        {...(profile === undefined ? {} : { profile })}
      />
      <div className="section-picker">
        <SectionPicker
          closeLabel={copy.close}
          item={section}
          pathname={pathname}
          onNavigate={navigate}
        />
      </div>
    </>
  );
}
{% endraw -%}

{% raw %}
import { useEffect, useRef, useState } from "react";
import {
  AppNavigation,
  SectionPicker,
  type NavigationIcon,
} from "@baukit/navigation/web";
import type { NavigationItem, NavigationProfile } from "@baukit/navigation";
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
const section: NavigationItem<NavigationIcon> = {
  id: "workspace",
  label: "Workspace",
  href: "/#items-title",
  icon: itemsIcon,
  children: [
    { id: "items", label: "Items", href: "/#items-title" },
    { id: "privacy", label: "Privacy", href: "/#privacy-title" },
  ],
};
const items: readonly NavigationItem<NavigationIcon>[] = [
  { id: "home", label: "Home", href: "/", icon: homeIcon },
  section,
];

export function NavigationShell({
  profile,
}: {
  readonly profile?: NavigationProfile;
}) {
  const [pathname, setPathname] = useState(
    window.location.pathname + window.location.hash,
  );
  const routeFocus = useRef<ReturnType<
    typeof createRouteFocusController
  > | null>(null);
  useEffect(() => {
    const controller = createRouteFocusController();
    routeFocus.current = controller;
    const update = () => {
      setPathname(window.location.pathname + window.location.hash);
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
  }
  return (
    <>
      <AppNavigation
        items={items}
        pathname={pathname}
        onNavigate={navigate}
        {...(profile === undefined ? {} : { profile })}
      />
      <div className="section-picker">
        <SectionPicker
          item={section}
          pathname={pathname}
          onNavigate={navigate}
        />
      </div>
    </>
  );
}
{% endraw %}

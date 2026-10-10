{% raw %}
import { Text, useWindowDimensions } from "react-native";
import { usePathname, useRouter } from "expo-router";
import {
  AppNavigation,
  SectionPicker,
  type NavigationIcon,
  type NavigationIconState,
  type NavigationTheme,
} from "@baukit/navigation/native";
import type { NavigationItem, NavigationProfile } from "@baukit/navigation";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "./theme";
import { useTranslation } from "react-i18next";
import { blendColors } from "@baukit/ui-tokens";

function NavigationGlyph({
  active,
  size,
  glyph,
}: NavigationIconState & { readonly glyph: string }) {
  const { theme } = useTheme();
  return (
    <Text
      style={{
        fontSize: size,
        color: active ? theme.color.accent : theme.color.text,
      }}
    >
      {glyph}
    </Text>
  );
}
const homeIcon: NavigationIcon = (state) => (
  <NavigationGlyph {...state} glyph="⌂" />
);
const itemsIcon: NavigationIcon = (state) => (
  <NavigationGlyph {...state} glyph="☷" />
);
function useNavigationItems() {
  const { t } = useTranslation("navigation");
  const workspace: NavigationItem<NavigationIcon> = {
    id: "workspace",
    label: t("workspace"),
    href: "/workspace/items",
    icon: itemsIcon,
    children: [
      { id: "items", label: t("items"), href: "/workspace/items" },
      { id: "privacy", label: t("privacy"), href: "/workspace/privacy" },
    ],
  };
  const items: readonly NavigationItem<NavigationIcon>[] = [
    { id: "home", label: t("today"), href: "/", icon: homeIcon },
    workspace,
  ];
  const labels = {
    label: t("primary"),
    collapseLabel: t("collapse"),
    expandLabel: t("expand"),
    closeLabel: t("close"),
  };
  return { items, workspace, labels };
}

function useNavigationTheme(): NavigationTheme {
  const { theme } = useTheme();
  return {
    background: theme.color.surface,
    text: theme.color.text,
    muted: theme.color.muted,
    danger: theme.color.error,
    activeBackground: blendColors(theme.color.accent, theme.color.surface, 0.14),
    activeText: theme.color.accent,
    ancestorText: theme.color.accent,
    border: theme.color.border,
    focus: theme.color.focus,
    spacing: theme.space.small,
    radius: theme.radius.button,
  };
}
export function NavigationBar({
  profile,
}: {
  readonly profile?: NavigationProfile;
}) {
  const { items, labels } = useNavigationItems();
  const router = useRouter();
  const pathname = usePathname();
  const theme = useNavigationTheme();
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  return (
    <AppNavigation
      {...labels}
      items={items}
      pathname={pathname}
      onNavigate={(href) => {
        router.navigate(href);
      }}
      theme={theme}
      insets={insets}
      width={width}
      {...(profile === undefined ? {} : { profile })}
    />
  );
}
export function WorkspacePicker() {
  const { workspace, labels } = useNavigationItems();
  const router = useRouter();
  const pathname = usePathname();
  const theme = useNavigationTheme();
  return (
    <SectionPicker
      item={workspace}
      closeLabel={labels.closeLabel}
      pathname={pathname}
      onNavigate={(href) => {
        router.navigate(href);
      }}
      theme={theme}
    />
  );
}
{% endraw -%}

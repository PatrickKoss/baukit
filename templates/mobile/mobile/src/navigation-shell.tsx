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
export const workspaceNavigation: NavigationItem<NavigationIcon> = {
  id: "workspace",
  label: "Workspace",
  href: "/workspace/items",
  icon: itemsIcon,
  children: [
    { id: "items", label: "Items", href: "/workspace/items" },
    { id: "privacy", label: "Privacy", href: "/workspace/privacy" },
  ],
};
const items: readonly NavigationItem<NavigationIcon>[] = [
  { id: "home", label: "Today", href: "/", icon: homeIcon },
  workspaceNavigation,
];

function useNavigationTheme(): NavigationTheme {
  const { theme } = useTheme();
  return {
    background: theme.color.surface,
    text: theme.color.text,
    muted: theme.color.muted,
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
  const router = useRouter();
  const pathname = usePathname();
  const theme = useNavigationTheme();
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  return (
    <AppNavigation
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
  const router = useRouter();
  const pathname = usePathname();
  const theme = useNavigationTheme();
  return (
    <SectionPicker
      item={workspaceNavigation}
      pathname={pathname}
      onNavigate={(href) => {
        router.navigate(href);
      }}
      theme={theme}
    />
  );
}
{% endraw %}

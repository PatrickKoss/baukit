{% raw %}
import { Tabs } from "expo-router";
import { useWindowDimensions } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { getNavigationLayout } from "@baukit/navigation";

import { NavigationBar } from "../../src/navigation-shell";
import { useTheme } from "../../src/theme";

export default function TabLayout() {
  const { theme } = useTheme();
  const { width } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  return (
    <Tabs
      tabBar={() => <NavigationBar />}
      screenOptions={{
        headerShown: false,
        sceneStyle: { backgroundColor: theme.color.background, paddingTop: insets.top },
        tabBarPosition:
          getNavigationLayout(width) === "rail" ? "left" : "bottom",
      }}
    >
      <Tabs.Screen name="index" options={{ title: "Today" }} />
      <Tabs.Screen name="workspace" options={{ title: "Workspace" }} />
    </Tabs>
  );
}
{% endraw %}

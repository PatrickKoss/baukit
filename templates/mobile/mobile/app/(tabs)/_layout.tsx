import { Tabs } from 'expo-router';

import { useTheme } from '../../src/theme';

const todayOptions = { title: 'Today' };

export default function TabLayout() {
  const { theme } = useTheme();
  const screenOptions = {
    headerShown: false,
    sceneStyle: { backgroundColor: theme.color.background },
    tabBarActiveTintColor: theme.color.accent,
    tabBarInactiveTintColor: theme.color.muted,
    tabBarStyle: {
      backgroundColor: theme.color.surface,
      borderTopColor: theme.color.border,
    },
  };
  return (
    <Tabs screenOptions={screenOptions}>
      <Tabs.Screen name="index" options={todayOptions} />
    </Tabs>
  );
}

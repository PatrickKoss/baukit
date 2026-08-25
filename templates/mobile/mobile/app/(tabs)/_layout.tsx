import { Tabs } from 'expo-router';

import { theme } from '../../src/theme';

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
const todayOptions = { title: 'Today' };

export default function TabLayout() {
  return (
    <Tabs screenOptions={screenOptions}>
      <Tabs.Screen name="index" options={todayOptions} />
    </Tabs>
  );
}

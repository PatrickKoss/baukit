import { Stack } from 'expo-router';

import { AppShell } from '../src/app-shell';
import { useTheme } from '../src/theme';

const tabOptions = { headerShown: false };

export default function RootLayout() {
  return (
    <AppShell>
      <RootNavigator />
    </AppShell>
  );
}

function RootNavigator() {
  const { theme } = useTheme();
  const screenOptions = {
    contentStyle: { backgroundColor: theme.color.background },
    headerStyle: { backgroundColor: theme.color.surface },
    headerTintColor: theme.color.text,
  };
  return (
    <Stack screenOptions={screenOptions}>
      <Stack.Screen name="(tabs)" options={tabOptions} />
    </Stack>
  );
}

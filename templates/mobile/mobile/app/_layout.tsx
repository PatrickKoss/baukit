import { Stack } from 'expo-router';

import { AppShell } from '../src/app-shell';
import { theme } from '../src/theme';

const screenOptions = {
  contentStyle: { backgroundColor: theme.color.background },
  headerStyle: { backgroundColor: theme.color.surface },
  headerTintColor: theme.color.text,
};
const tabOptions = { headerShown: false };

export default function RootLayout() {
  return (
    <AppShell>
      <Stack screenOptions={screenOptions}>
        <Stack.Screen name="(tabs)" options={tabOptions} />
      </Stack>
    </AppShell>
  );
}

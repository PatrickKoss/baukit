import { Stack } from 'expo-router';

import { AppShell } from '../src/app-shell';
import { OidcAuthProvider, useOidcAuth } from '../src/auth';
import { AuthenticatedLocalDataProvider } from '../src/local-data';
import { theme } from '../src/theme';

const screenOptions = {
  contentStyle: { backgroundColor: theme.color.background },
  headerStyle: { backgroundColor: theme.color.surface },
  headerTintColor: theme.color.text,
};
const groupOptions = { headerShown: false };

export default function RootLayout() {
  return (
    <OidcAuthProvider>
      <AuthGate />
    </OidcAuthProvider>
  );
}

function AuthGate() {
  const auth = useOidcAuth();
  return (
    <AppShell preferenceSubjectId={auth.subject ?? null}>
      <AuthenticatedLocalDataProvider sessionExpired={auth.sessionExpired} subject={auth.subject}>
        <Stack screenOptions={screenOptions}>
          <Stack.Protected guard={auth.accessToken === undefined}>
            <Stack.Screen name="(auth)" options={groupOptions} />
          </Stack.Protected>
          <Stack.Protected guard={auth.accessToken !== undefined}>
            <Stack.Screen name="(tabs)" options={groupOptions} />
          </Stack.Protected>
        </Stack>
      </AuthenticatedLocalDataProvider>
    </AppShell>
  );
}

import { Stack } from 'expo-router';

import { AppShell } from '../src/app-shell';
import { OidcAuthProvider, useOidcAuth } from '../src/auth';
import { AuthenticatedLocalDataProvider } from '../src/local-data';
import { useTheme } from '../src/theme';

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
      <AppNavigator auth={auth} />
    </AppShell>
  );
}

function AppNavigator({
  auth,
}: {
  readonly auth: ReturnType<typeof useOidcAuth>;
}) {
  const { theme } = useTheme();
  const screenOptions = {
    contentStyle: { backgroundColor: theme.color.background },
    headerStyle: { backgroundColor: theme.color.surface },
    headerTintColor: theme.color.text,
  };
  return (
    <AuthenticatedLocalDataProvider
      sessionExpired={auth.sessionExpired}
      subject={auth.subject}
    >
      <Stack screenOptions={screenOptions}>
        <Stack.Protected guard={auth.accessToken === undefined}>
          <Stack.Screen name="(auth)" options={groupOptions} />
        </Stack.Protected>
        <Stack.Protected guard={auth.accessToken !== undefined}>
          <Stack.Screen name="(tabs)" options={groupOptions} />
        </Stack.Protected>
      </Stack>
    </AuthenticatedLocalDataProvider>
  );
}

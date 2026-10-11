import { Text } from 'react-native';
import { Stack } from 'expo-router';

import { AppShell } from '../src/app-shell';
import { AuthProvider, useAuth } from '../src/auth';
import { AuthenticatedLocalDataProvider } from '../src/local-data';
import { useServerProfile } from '../src/server-profile';
import { useTheme } from '../src/theme';

const groupOptions = { headerShown: false };

export default function RootLayout() {
  return (
    <AuthProvider>
      <AuthGate />
    </AuthProvider>
  );
}

function AuthGate() {
  const auth = useAuth();
  const { profile, error } = useServerProfile(auth.accessToken, auth.subject);
  return (
    <AppShell preferenceSubjectId={profile?.id ?? null}>
      <AppNavigator auth={auth} profileId={profile?.id} profileError={error} />
    </AppShell>
  );
}

function AppNavigator({
  auth,
  profileId,
  profileError,
}: {
  readonly auth: ReturnType<typeof useAuth>;
  readonly profileId: string | undefined;
  readonly profileError: Error | undefined;
}) {
  const { theme } = useTheme();
  const screenOptions = {
    contentStyle: { backgroundColor: theme.color.background },
    headerStyle: { backgroundColor: theme.color.surface },
    headerTintColor: theme.color.text,
  };
  return (
    <AuthenticatedLocalDataProvider
      sessionExpiredReason={auth.sessionExpiredReason}
      profileId={profileId}
    >
      {profileError === undefined ? null : <Text accessibilityRole="alert">{profileError.message}</Text>}
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

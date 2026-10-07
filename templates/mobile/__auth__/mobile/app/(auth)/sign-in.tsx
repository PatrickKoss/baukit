import { ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { ActionButton } from '../../src/action-button';
import { useAuth } from '../../src/auth';
import { useTheme, type AppTheme } from '../../src/theme';

import { PRODUCT_NAME } from '../../src/product';

export default function SignInScreen() {
  const auth = useAuth();
  const insets = useSafeAreaInsets();
  const { mode, theme } = useTheme();
  const styles = createStyles(theme);
  return (
    <View style={[styles.safeArea, { paddingTop: insets.top }]} testID="sign-in-screen">
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text style={styles.title}>{PRODUCT_NAME}</Text>
        <Text style={styles.subtitle}>Sign in to your account.</Text>

        <View style={styles.card}>
          <Text style={styles.sectionTitle}>Sign in</Text>
          {auth.error === undefined ? null : <Text style={styles.error}>{auth.error}</Text>}
          {auth.announcement === undefined ? null : (
            <Text accessibilityLiveRegion="polite" style={styles.muted}>
              {auth.announcement}
            </Text>
          )}
          <ActionButton
            disabled={!auth.ready}
            label={auth.ready ? 'Sign in with {{ "local Keycloak" if context.auth_oidc else "Clerk" if context.auth_clerk else "WorkOS" }}' : 'Preparing sign in...'}
            onPress={() => void auth.signIn(mode)}
          />
        </View>
      </ScrollView>
    </View>
  );
}

function createStyles(theme: AppTheme) {
  return StyleSheet.create({
    safeArea: { flex: 1, backgroundColor: theme.color.background },
    page: { gap: theme.space.medium, padding: theme.space.large },
    eyebrow: { color: theme.color.accent, fontSize: 12, fontWeight: '700', letterSpacing: 1.5 },
    title: { color: theme.color.text, fontSize: 34, fontWeight: '700' },
    subtitle: { color: theme.color.muted, fontSize: 16 },
    card: {
      gap: theme.space.small,
      padding: theme.space.medium,
      backgroundColor: theme.color.surface,
      borderRadius: theme.radius.card,
    },
    sectionTitle: { color: theme.color.text, fontSize: 18, fontWeight: '700' },
    error: { color: theme.color.error },
    muted: { color: theme.color.muted, lineHeight: 21 },
  });
}

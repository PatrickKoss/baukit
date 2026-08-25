import { ScrollView, StyleSheet, Text, View } from 'react-native';

import { ActionButton } from '../../src/action-button';
import { useOidcAuth } from '../../src/auth';
import { theme } from '../../src/theme';

export default function SignInScreen() {
  const auth = useOidcAuth();
  return (
    <View style={styles.safeArea}>
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text style={styles.title}>{{ context.app_name }}</Text>
        <Text style={styles.subtitle}>Standard OIDC discovery + authorization code PKCE</Text>

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
            label={auth.ready ? 'Sign in with local Keycloak' : 'Preparing sign in…'}
            onPress={() => void auth.signIn()}
          />
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
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

import { useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import {
  isPersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from '@baukit/data-contracts';

import { loadAnalytics } from './src/analytics';
import type { ProductEvent } from './src/analytics-client';
import { currentUser, listItems } from './src/api';
import type { CurrentUser, Item } from './src/api';
import { useOidcAuth } from './src/auth';
import { useAuthenticatedLocalData } from './src/local-data';
import { theme } from './src/theme';
import type { AnalyticsClient, ConsentState } from '@baukit/analytics-core';

export default function App() {
  const auth = useOidcAuth();
  const localData = useAuthenticatedLocalData(auth.subject, auth.sessionExpired);
  const blockIdentityMismatch = localData.blockIdentityMismatch;
  const [items, setItems] = useState<readonly Item[]>([]);
  const [user, setUser] = useState<CurrentUser>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [analytics, setAnalytics] = useState<AnalyticsClient<ProductEvent>>();
  const [consent, setConsent] = useState<ConsentState>('unknown');

  useEffect(() => {
    let active = true;
    void loadAnalytics().then((client) => {
      if (active) {
        setAnalytics(client);
        setConsent(client.consent);
      }
    });
    return () => {
      active = false;
    };
  }, []);

  const partition =
    localData.state.status === 'ready' &&
    localData.state.partition.subject === auth.subject &&
    !auth.sessionExpired
      ? localData.state.partition
      : undefined;

  useEffect(() => {
    let active = true;
    if (partition === undefined) {
      void Promise.resolve().then(() => {
        if (active) {
          setItems([]);
          setUser(undefined);
          setLoading(false);
        }
      });
      return () => {
        active = false;
      };
    }
    void Promise.resolve().then(() => {
      if (active) {
        setLoading(true);
        setError(undefined);
      }
    });
    void Promise.all([listItems(), currentUser()])
      .then(async ([nextItems, nextUser]) => {
        await recheckServerSubjectBeforeSyncAdoption({
          partitionSubject: partition.subject,
          readServerSubject: () => Promise.resolve(nextUser.subject),
          adopt: async () => {
            const client = await loadAnalytics();
            if (active) {
              client.identify(nextUser.id);
              setItems(nextItems);
              setUser(nextUser);
            }
          },
        });
      })
      .catch(async (cause: unknown) => {
        if (isPersistenceIdentityMismatchError(cause)) {
          try {
            await blockIdentityMismatch(cause);
          } catch {
            if (active) setError('Could not safely close local account data.');
            return;
          }
        }
        if (active) {
          setError(
            isPersistenceIdentityMismatchError(cause)
              ? 'The server account does not match this local data partition.'
              : cause instanceof Error
                ? cause.message
                : 'Could not load authenticated data.',
          );
        }
      })
      .finally(() => {
        if (active) {
          setLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [blockIdentityMismatch, partition]);

  function chooseConsent(nextConsent: ConsentState): void {
    if (analytics === undefined) {
      return;
    }
    analytics.setConsent(nextConsent);
    setConsent(nextConsent);
    if (nextConsent === 'granted') {
      analytics.capture({ name: 'items_viewed', properties: { count: items.length } });
    }
  }

  return (
    <View style={styles.safeArea}>
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text style={styles.title}>{{ context.app_name }}</Text>
        <Text style={styles.subtitle}>Standard OIDC discovery + authorization code PKCE</Text>

        <View style={styles.card}>
          <Text style={styles.sectionTitle}>Identity</Text>
          {auth.error === undefined ? null : <Text style={styles.error}>{auth.error}</Text>}
          {localData.state.status === 'blocked' ? (
            <Text accessibilityLiveRegion="assertive" style={styles.error}>
              Local data is blocked. Sign in again or contact support before continuing.
            </Text>
          ) : null}
          {auth.announcement === undefined ? null : (
            <Text accessibilityLiveRegion="polite" style={styles.muted}>
              {auth.announcement}
            </Text>
          )}
          {auth.accessToken === undefined ? (
            <ActionButton
              disabled={!auth.ready}
              label={auth.ready ? 'Sign in with local Keycloak' : 'Preparing sign in…'}
              onPress={() => void auth.signIn()}
            />
          ) : (
            <>
              <Text style={styles.muted}>
                OIDC subject {auth.subject}; backend subject {user?.subject ?? 'loading…'}; internal
                user ID {user?.id ?? 'loading…'}.
              </Text>
              <Text style={styles.muted}>Local data: {localData.state.status}.</Text>
              <ActionButton label="Sign out" onPress={() => void auth.signOut()} secondary />
            </>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle}>Items</Text>
          {loading || localData.state.status === 'initializing' ? (
            <ActivityIndicator
              accessibilityHint="Wait until the authenticated local partition is ready"
              accessibilityLabel="Preparing local data"
              color={theme.color.accent}
            />
          ) : null}
          {error === undefined ? null : <Text style={styles.error}>{error}</Text>}
          {localData.state.status === 'signed-out' ? (
            <Text style={styles.muted}>Sign in to open your local data partition.</Text>
          ) : null}
          {!loading && localData.state.status === 'ready' && error === undefined && items.length === 0 ? (
            <Text style={styles.muted}>No items yet.</Text>
          ) : null}
          {items.map((item) => (
            <View key={item.id} style={styles.item}>
              <Text style={styles.itemName}>{item.name}</Text>
              <Text style={styles.itemId}>{item.id}</Text>
            </View>
          ))}
        </View>

        <View style={styles.settings}>
          <Text style={styles.sectionTitle}>Analytics privacy</Text>
          <Text style={styles.muted}>Current consent: {consent}. No events leave the app.</Text>
          <View style={styles.actions}>
            <ActionButton
              label="Allow"
              onPress={() => {
                chooseConsent('granted');
              }}
            />
            <ActionButton
              label="Deny"
              onPress={() => {
                chooseConsent('denied');
              }}
              secondary
            />
          </View>
        </View>
      </ScrollView>
    </View>
  );
}

interface ActionButtonProps {
  readonly disabled?: boolean;
  readonly label: string;
  readonly onPress: () => void;
  readonly secondary?: boolean;
}

function ActionButton({ disabled = false, label, onPress, secondary = false }: ActionButtonProps) {
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={[
        styles.button,
        secondary ? styles.buttonSecondary : undefined,
        disabled ? styles.buttonDisabled : undefined,
      ]}
    >
      <Text style={[styles.buttonText, secondary ? styles.buttonTextSecondary : undefined]}>
        {label}
      </Text>
    </Pressable>
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
  item: { gap: 3, paddingVertical: theme.space.small },
  itemName: { color: theme.color.text, fontSize: 17, fontWeight: '600' },
  itemId: { color: theme.color.muted, fontSize: 12 },
  error: { color: theme.color.error },
  muted: { color: theme.color.muted, lineHeight: 21 },
  settings: {
    gap: theme.space.small,
    padding: theme.space.medium,
    borderColor: theme.color.border,
    borderRadius: theme.radius.card,
    borderWidth: 1,
  },
  sectionTitle: { color: theme.color.text, fontSize: 18, fontWeight: '700' },
  actions: { flexDirection: 'row', gap: theme.space.small, marginTop: theme.space.small },
  button: {
    paddingHorizontal: theme.space.medium,
    paddingVertical: theme.space.small,
    backgroundColor: theme.color.accent,
    borderRadius: theme.radius.button,
  },
  buttonDisabled: { opacity: 0.5 },
  buttonSecondary: { backgroundColor: theme.color.surface },
  buttonText: { color: theme.color.onAccent, fontWeight: '700' },
  buttonTextSecondary: { color: theme.color.text },
});

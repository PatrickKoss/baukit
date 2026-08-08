import { useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';

import { loadAnalytics } from './src/analytics';
import { listItems, type Item } from './src/api';
import { theme } from './src/theme';
import type { AnalyticsClient, ConsentState } from '@baukit/analytics-core';
import type { ProductEvent } from './src/analytics-client';

export default function App() {
  const [items, setItems] = useState<readonly Item[]>([]);
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

  useEffect(() => {
    let active = true;
    void listItems()
      .then((nextItems) => {
        if (active) {
          setItems(nextItems);
          setError(undefined);
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(cause instanceof Error ? cause.message : 'Could not load items.');
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
  }, []);

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
        <Text style={styles.subtitle}>Items from the shared backend API</Text>

        <View style={styles.card}>
          {loading ? <ActivityIndicator color={theme.color.accent} /> : null}
          {error === undefined ? null : <Text style={styles.error}>{error}</Text>}
          {!loading && error === undefined && items.length === 0 ? (
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
          <Text style={styles.muted}>
            Current consent: {consent}. Analytics uses a no-op transport until you configure a
            provider.
          </Text>
          <View style={styles.actions}>
            <ConsentButton
              label="Allow"
              onPress={() => {
                chooseConsent('granted');
              }}
            />
            <ConsentButton
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

interface ConsentButtonProps {
  readonly label: string;
  readonly onPress: () => void;
  readonly secondary?: boolean;
}

function ConsentButton({ label, onPress, secondary = false }: ConsentButtonProps) {
  return (
    <Pressable
      accessibilityRole="button"
      onPress={onPress}
      style={[styles.button, secondary ? styles.buttonSecondary : undefined]}
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
  buttonSecondary: { backgroundColor: theme.color.surface },
  buttonText: { color: theme.color.onAccent, fontWeight: '700' },
  buttonTextSecondary: { color: theme.color.text },
});

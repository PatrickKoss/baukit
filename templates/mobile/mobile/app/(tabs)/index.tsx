import { useEffect, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useTranslation } from 'react-i18next';

import { ActionButton } from '../../src/action-button';
import { useAnalyticsConsent } from '../../src/app-shell';
import { listItems, type Item } from '../../src/api';
import { theme } from '../../src/theme';
import type { ConsentState } from '@baukit/analytics-core';

export default function TodayScreen() {
  const { t } = useTranslation(['bootstrap', 'home']);
  const { analytics, consent, setConsent } = useAnalyticsConsent();
  const [items, setItems] = useState<readonly Item[]>([]);
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);

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

  async function chooseConsent(nextConsent: ConsentState): Promise<void> {
    await setConsent(nextConsent);
    if (nextConsent === 'granted') {
      analytics?.capture({ name: 'items_viewed', properties: { count: items.length } });
    }
  }

  return (
    <View style={styles.safeArea}>
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text style={styles.title}>{{ context.app_name }}</Text>
        <Text style={styles.subtitle}>Items from the shared backend API</Text>

        <View style={styles.card}>
          <Text style={styles.sectionTitle}>{t('itemsTitle', { ns: 'home' })}</Text>
          {loading ? (
            <View style={styles.loading}>
              <ActivityIndicator color={theme.color.accent} />
              <Text style={styles.muted}>{t('loadingItems', { ns: 'bootstrap' })}</Text>
            </View>
          ) : null}
          {error === undefined ? null : <Text style={styles.error}>{error}</Text>}
          {!loading && error === undefined && items.length === 0 ? (
            <Text style={styles.muted}>{t('emptyItems', { ns: 'home' })}</Text>
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
            <ActionButton
              label="Allow"
              onPress={() => {
                void chooseConsent('granted');
              }}
            />
            <ActionButton
              label="Deny"
              onPress={() => {
                void chooseConsent('denied');
              }}
              secondary
            />
          </View>
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
  loading: { alignItems: 'center', flexDirection: 'row', gap: theme.space.small },
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
});

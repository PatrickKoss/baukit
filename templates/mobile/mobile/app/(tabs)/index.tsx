import { type ReactElement, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Platform, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useTranslation } from 'react-i18next';

import { ActionButton } from '../../src/action-button';
import { useAnalyticsConsent } from '../../src/app-shell';
import { listItems, type Item } from '../../src/api';
import { checkText } from '../../src/limits';
import { useRouteHeadingFocus } from '../../src/route-heading-focus';
import { useTheme, type AppTheme } from '../../src/theme';
import { ThemeModeControl } from '../../src/theme-mode-control';
import type { ConsentState } from '@baukit/analytics-core';

import { PRODUCT_NAME } from '../../src/product';

const routeHeadingFocusProps = Platform.OS === 'web' ? { tabIndex: -1 } : {};

type Styles = ReturnType<typeof createStyles>;

interface ItemsState {
  readonly items: readonly Item[];
  readonly error: string | undefined;
  readonly loading: boolean;
}

function useItems(): ItemsState {
  const [items, setItems] = useState<readonly Item[]>([]);
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let active = true;
    void listItems()
      .then((nextItems) => {
        if (active) {
          nextItems.forEach((item) => {
            checkText('item.name', item.name);
          });
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

  return { items, error, loading };
}

function ItemsCard({
  state,
  styles,
  accent,
}: {
  readonly state: ItemsState;
  readonly styles: Styles;
  readonly accent: string;
}): ReactElement {
  const { t } = useTranslation(['bootstrap', 'home']);
  const { items, error, loading } = state;
  return (
    <View style={styles.card}>
      <Text style={styles.sectionTitle}>{t('itemsTitle', { ns: 'home' })}</Text>
      {loading ? (
        <View style={styles.loading}>
          <ActivityIndicator color={accent} />
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
  );
}

function AnalyticsPrivacy({
  itemCount,
  styles,
}: {
  readonly itemCount: number;
  readonly styles: Styles;
}): ReactElement {
  const { analytics, consent, setConsent } = useAnalyticsConsent();

  async function chooseConsent(nextConsent: ConsentState): Promise<void> {
    await setConsent(nextConsent);
    if (nextConsent === 'granted') {
      analytics?.capture({ name: 'items_viewed', properties: { count: itemCount } });
    }
  }

  return (
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
  );
}

export default function TodayScreen(): ReactElement {
  const { theme } = useTheme();
  const styles = createStyles(theme);
  const itemsState = useItems();
  const headingRef = useRef<Text>(null);

  useRouteHeadingFocus(headingRef, !itemsState.loading);

  return (
    <View style={styles.safeArea}>
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text
          ref={headingRef}
          accessibilityRole="header"
          style={styles.title}
          {...routeHeadingFocusProps}
        >
          {PRODUCT_NAME}
        </Text>
        <Text style={styles.subtitle}>Items from the shared backend API</Text>

        <ItemsCard state={itemsState} styles={styles} accent={theme.color.accent} />

        <View style={styles.settings}>
          <Text style={styles.sectionTitle}>Appearance</Text>
          <ThemeModeControl />
        </View>

        <AnalyticsPrivacy itemCount={itemsState.items.length} styles={styles} />
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
}

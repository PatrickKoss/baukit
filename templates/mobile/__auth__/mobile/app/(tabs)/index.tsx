import { type ReactElement, useEffect, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';
import {
  isPersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from '@baukit/data-contracts';
import { useTranslation } from 'react-i18next';
import { Link } from 'expo-router';

import { ActionButton } from '../../src/action-button';
import { loadAnalytics } from '../../src/analytics';
import { useAnalyticsConsent, useAppPreferences } from '../../src/app-shell';
import { currentUser, listItems } from '../../src/api';
import type { CurrentUser, Item } from '../../src/api';
import { useAuth } from '../../src/auth';
import { useAuthenticatedLocalData } from '../../src/local-data';
import { signOutWithPreferenceReset } from '../../src/preference-sign-out';
import { useTheme, type AppTheme } from '../../src/theme';
import { ThemeModeControl } from '../../src/theme-mode-control';
import type { ConsentState } from '@baukit/analytics-core';
import { PRODUCT_NAME } from '../../src/product';

type Styles = ReturnType<typeof createStyles>;
type LocalData = ReturnType<typeof useAuthenticatedLocalData>;
type Partition = Extract<LocalData['state'], { status: 'ready' }>['partition'];

interface AuthenticatedItems {
  readonly items: readonly Item[];
  readonly user: CurrentUser | undefined;
  readonly error: string | undefined;
  readonly loading: boolean;
}

function loadErrorMessage(cause: unknown): string {
  if (isPersistenceIdentityMismatchError(cause)) {
    return 'The server account does not match this local data partition.';
  }
  return cause instanceof Error ? cause.message : 'Could not load authenticated data.';
}

function useAuthenticatedItems(
  partition: Partition | undefined,
  blockIdentityMismatch: LocalData['blockIdentityMismatch'],
): AuthenticatedItems {
  const [items, setItems] = useState<readonly Item[]>([]);
  const [user, setUser] = useState<CurrentUser>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);

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
          readServerSubject: () => Promise.resolve(nextUser.id),
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
        if (active) setError(loadErrorMessage(cause));
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

  return { items, user, error, loading };
}

function IdentityCard({
  localData,
  user,
  styles,
}: {
  readonly localData: LocalData;
  readonly user: CurrentUser | undefined;
  readonly styles: Styles;
}): ReactElement {
  const { t } = useTranslation(['home']);
  const auth = useAuth();
  const { resetPreferenceIdentity } = useAppPreferences();
  return (
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
      <Text style={styles.muted}>
        OIDC subject {auth.subject}; backend subject {user?.subject ?? 'loading…'}; internal user
        ID {user?.id ?? 'loading…'}.
      </Text>
      <Text style={styles.muted}>Local data: {localData.state.status}.</Text>
      <ActionButton
        label="Sign out"
        onPress={() =>
          void signOutWithPreferenceReset({
            resetPreferenceIdentity,
            signOut: auth.signOut,
          })
        }
        secondary
      />
      <Link href="/delete-profile" accessibilityRole="link" style={styles.muted}>
        {t('erasure.title', { ns: 'home' })}
      </Link>
    </View>
  );
}

function ItemsCard({
  data,
  localStatus,
  styles,
  accent,
}: {
  readonly data: AuthenticatedItems;
  readonly localStatus: LocalData['state']['status'];
  readonly styles: Styles;
  readonly accent: string;
}): ReactElement {
  const { t } = useTranslation(['bootstrap', 'home']);
  const { items, error, loading } = data;
  const empty = !loading && localStatus === 'ready' && error === undefined && items.length === 0;
  return (
    <View style={styles.card}>
      <Text style={styles.sectionTitle}>{t('itemsTitle', { ns: 'home' })}</Text>
      {loading || localStatus === 'initializing' ? (
        <View style={styles.loading}>
          <ActivityIndicator
            accessibilityHint="Wait until the authenticated local partition is ready"
            accessibilityLabel="Preparing local data"
            color={accent}
          />
          <Text style={styles.muted}>{t('loadingItems', { ns: 'bootstrap' })}</Text>
        </View>
      ) : null}
      {error === undefined ? null : <Text style={styles.error}>{error}</Text>}
      {empty ? <Text style={styles.muted}>{t('emptyItems', { ns: 'home' })}</Text> : null}
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
      <Text style={styles.muted}>Current consent: {consent}. No events leave the app.</Text>
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
  const auth = useAuth();
  const localData = useAuthenticatedLocalData();
  const partition =
    localData.state.status === 'ready' &&
    auth.accessToken !== undefined &&
    !auth.sessionExpired
      ? localData.state.partition
      : undefined;
  const data = useAuthenticatedItems(partition, localData.blockIdentityMismatch);

  return (
    <View style={styles.safeArea}>
      <ScrollView contentContainerStyle={styles.page}>
        <Text style={styles.eyebrow}>BAUKIT MOBILE</Text>
        <Text style={styles.title}>{PRODUCT_NAME}</Text>
        <Text style={styles.subtitle}>Standard OIDC discovery + authorization code PKCE</Text>

        <IdentityCard localData={localData} user={data.user} styles={styles} />
        <ItemsCard
          data={data}
          localStatus={localData.state.status}
          styles={styles}
          accent={theme.color.accent}
        />

        <View style={styles.settings}>
          <Text style={styles.sectionTitle}>Appearance</Text>
          <ThemeModeControl />
        </View>

        <AnalyticsPrivacy itemCount={data.items.length} styles={styles} />
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

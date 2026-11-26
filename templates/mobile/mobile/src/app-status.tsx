import { Component, useEffect, type ReactElement, type ReactNode } from 'react';
import { announce } from '@baukit/a11y-core';
import { useTranslation } from 'react-i18next';
import { StyleSheet, Text, View } from 'react-native';

import { ActionButton } from './action-button';
import { apiAvailability, type ApiAvailabilityStore } from './api-availability';
import { formatRetryTime, useApiAvailability } from './api-status';
import { useTheme, type AppTheme } from './theme';

const CRASHED_TITLE_FONT_SIZE = 20;

/** Announces the API cooldown politely; renders nothing while the API is reachable. */
export function ApiUnavailableBanner({
  store = apiAvailability,
}: {
  readonly store?: ApiAvailabilityStore;
}): ReactElement | null {
  const retryAt = useApiAvailability(store);
  const { t, i18n } = useTranslation('status');
  const { theme } = useTheme();
  const message =
    retryAt === undefined
      ? undefined
      : t('unavailable').replace('{time}', formatRetryTime(retryAt, i18n.language));
  useEffect(() => {
    if (message !== undefined) announce(message);
  }, [message]);
  if (message === undefined) return null;
  const styles = createStyles(theme);
  return (
    <View accessible style={styles.banner}>
      <Text style={styles.bannerText}>{message}</Text>
    </View>
  );
}

interface ErrorBoundaryState {
  readonly failed: boolean;
}

export class AppErrorBoundary extends Component<{ readonly children: ReactNode }, ErrorBoundaryState> {
  public override state: ErrorBoundaryState = { failed: false };

  public static getDerivedStateFromError(): ErrorBoundaryState {
    return { failed: true };
  }

  public override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return (
      <AppCrashed
        onRetry={() => {
          this.setState({ failed: false });
        }}
      />
    );
  }
}

function AppCrashed({ onRetry }: { readonly onRetry: () => void }): ReactElement {
  const { t } = useTranslation('status');
  const { theme } = useTheme();
  const title = t('crashed');
  useEffect(() => {
    announce(title, { assertive: true });
  }, [title]);
  const styles = createStyles(theme);
  return (
    <View style={styles.crashed}>
      <Text accessibilityRole="header" style={styles.crashedTitle}>
        {title}
      </Text>
      <Text style={styles.bannerText}>{t('crashedDetail')}</Text>
      <ActionButton label={t('retry')} onPress={onRetry} />
    </View>
  );
}

function createStyles(theme: AppTheme) {
  return StyleSheet.create({
    banner: {
      paddingHorizontal: theme.space.medium,
      paddingVertical: theme.space.small,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderColor: theme.color.border,
      backgroundColor: theme.color.surface,
    },
    bannerText: {
      color: theme.color.text,
    },
    crashed: {
      flex: 1,
      gap: theme.space.medium,
      justifyContent: 'center',
      padding: theme.space.large,
      backgroundColor: theme.color.background,
    },
    crashedTitle: {
      color: theme.color.text,
      fontSize: CRASHED_TITLE_FONT_SIZE,
      fontWeight: '700',
    },
  });
}

import { useEffect, useRef, useState } from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';
import { announce, focusAccessibilityElement } from '@baukit/a11y-core';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import { ApiError } from '@baukit/api-runtime';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';
import { useTranslation } from 'react-i18next';

import { ActionButton } from './action-button';
import { useTheme, type AppTheme } from './theme';

interface DeleteProfileScreenProps {
  readonly erase: () => Promise<ProductProfileErasureResult>;
  readonly poll?: (operationId: string, signal: AbortSignal) => Promise<ProfileErasureOperation>;
  readonly available?: boolean;
}

type State =
  'idle' | 'confirming' | 'erasing' | 'preparation-failure' | ProductProfileErasureResult;

export function DeleteProfileScreen({ erase, poll, available = true }: DeleteProfileScreenProps) {
  const { t } = useTranslation('home');
  const { theme } = useTheme();
  const styles = createStyles(theme);
  const [state, setState] = useState<State>('idle');
  const [operation, setOperation] = useState<ProfileErasureOperation>();
  const [statusError, setStatusError] = useState<'unavailable' | 'expired' | null>(null);
  const [pollRound, setPollRound] = useState(0);
  const [finishedRound, setFinishedRound] = useState<number | null>(null);
  const running = useRef(false);
  const heading = useRef<Text>(null);
  const status = useRef<Text>(null);
  const result = typeof state === 'string' ? undefined : state;
  const resultMessage =
    result === undefined
      ? undefined
      : t(
          {
            erased: 'erasure.erased',
            pending: 'erasure.pending',
            ambiguous: 'erasure.ambiguous',
            'server-failure': 'erasure.serverFailure',
            'local-failure': 'erasure.localFailure',
            'signout-failure': 'erasure.signoutFailure',
          }[result.status],
        );
  let message = state === 'erasing' ? t('erasure.busy') : resultMessage;
  if (state === 'preparation-failure') message = t('erasure.preparationFailure');
  if (statusError === 'unavailable') message = t('erasure.statusError');
  if (statusError === 'expired') message = t('erasure.statusExpired');
  if (operation?.status === 'completed') message = t('erasure.erased');
  if (operation?.status === 'failed') message = t('erasure.failed');
  const retryable =
    state === 'preparation-failure' ||
    result?.status === 'server-failure' ||
    result?.status === 'ambiguous';
  const pendingId = result?.status === 'pending' ? result.receipt.operationId : null;
  const checking = poll !== undefined && pendingId !== null && finishedRound !== pollRound;
  const canCheck =
    statusError !== 'expired' &&
    poll !== undefined &&
    pendingId !== null &&
    (operation === undefined || operation.status === 'pending');

  useEffect(() => {
    focusAccessibilityElement(message === undefined ? heading : status);
    if (message !== undefined) announce(message);
    else if (state === 'confirming') announce(t('erasure.confirmation'));
  }, [message, state, t]);

  useEffect(() => {
    if (poll === undefined || pendingId === null) return;
    const readStatus = poll;
    const operationId = pendingId;
    const controller = new AbortController();
    async function checkStatus(): Promise<void> {
      try {
        const next = await readStatus(operationId, controller.signal);
        if (!controller.signal.aborted) setOperation(next);
      } catch (cause) {
        if (!controller.signal.aborted) {
          setStatusError(
            cause instanceof ApiError && cause.status === 401 ? 'expired' : 'unavailable',
          );
        }
      } finally {
        if (!controller.signal.aborted) setFinishedRound(pollRound);
      }
    }
    void checkStatus();
    return () => {
      controller.abort();
    };
  }, [pendingId, poll, pollRound]);

  async function submit(): Promise<void> {
    if (running.current) return;
    running.current = true;
    setState('erasing');
    try {
      setState(await erase());
    } catch {
      setState('preparation-failure');
    } finally {
      running.current = false;
    }
  }

  return (
    <ScrollView contentContainerStyle={styles.content}>
      <Text ref={heading} accessibilityRole="header" style={styles.heading}>
        {t('erasure.title')}
      </Text>
      <Text style={styles.text}>
        {state === 'confirming' ? t('erasure.confirmation') : t('erasure.description')}
      </Text>
      {message === undefined ? null : (
        <Text ref={status} accessibilityLiveRegion="polite" style={styles.text}>
          {message}
        </Text>
      )}
      {state === 'idle' ? (
        <ActionButton
          disabled={!available}
          label={t('erasure.continue')}
          onPress={() => {
            setState('confirming');
          }}
          secondary
        />
      ) : null}
      {state === 'confirming' ? (
        <View style={styles.actions}>
          <ActionButton
            label={t('erasure.cancel')}
            onPress={() => {
              setState('idle');
            }}
            secondary
          />
          <ActionButton
            label={t('erasure.confirm')}
            onPress={() => {
              void submit();
            }}
          />
        </View>
      ) : null}
      {retryable ? (
        <ActionButton
          label={t('erasure.retry')}
          onPress={() => {
            void submit();
          }}
        />
      ) : null}
      {canCheck ? (
        <ActionButton
          disabled={checking}
          label={t('erasure.checkStatus')}
          onPress={() => {
            setStatusError(null);
            setPollRound((round) => round + 1);
          }}
          secondary
        />
      ) : null}
    </ScrollView>
  );
}

function createStyles(theme: AppTheme) {
  return StyleSheet.create({
    content: { padding: theme.space.medium, gap: theme.space.medium },
    actions: { gap: theme.space.small },
    heading: { color: theme.color.text, fontSize: 24, fontWeight: '700' },
    text: { color: theme.color.text, fontSize: 16 },
  });
}

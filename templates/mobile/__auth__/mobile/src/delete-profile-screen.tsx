import { useEffect, useRef, useState } from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';
import { announce, focusAccessibilityElement } from '@baukit/a11y-core';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';
import { useTranslation } from 'react-i18next';

import { ActionButton } from './action-button';
import { useTheme, type AppTheme } from './theme';

interface DeleteProfileScreenProps {
  readonly erase: () => Promise<ProductProfileErasureResult>;
  readonly poll?: (operationId: string, signal: AbortSignal) => Promise<ProfileErasureOperation>;
  readonly available?: boolean;
}

type State = 'idle' | 'confirming' | 'erasing' | ProductProfileErasureResult;

export function DeleteProfileScreen({ erase, poll, available = true }: DeleteProfileScreenProps) {
  const { t } = useTranslation('home');
  const { theme } = useTheme();
  const styles = createStyles(theme);
  const [state, setState] = useState<State>('idle');
  const [operation, setOperation] = useState<ProfileErasureOperation>();
  const [statusError, setStatusError] = useState(false);
  const [checking, setChecking] = useState(false);
  const running = useRef(false);
  const heading = useRef<Text>(null);
  const status = useRef<Text>(null);
  const polling = useRef<AbortController | null>(null);
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
  if (statusError) message = t('erasure.statusError');
  if (operation?.status === 'completed') message = t('erasure.erased');
  if (operation?.status === 'failed') message = t('erasure.failed');
  const retryable = result?.status === 'server-failure' || result?.status === 'ambiguous';
  const pendingId = result?.status === 'pending' ? result.receipt.operationId : null;
  const canCheck =
    poll !== undefined &&
    pendingId !== null &&
    (operation === undefined || operation.status === 'pending');

  useEffect(() => {
    focusAccessibilityElement(message === undefined ? heading : status);
    if (message !== undefined) announce(message);
    else if (state === 'confirming') announce(t('erasure.confirmation'));
  }, [message, state, t]);

  useEffect(
    () => () => {
      polling.current?.abort();
    },
    [],
  );

  async function submit(): Promise<void> {
    if (running.current) return;
    running.current = true;
    setState('erasing');
    try {
      setState(await erase());
    } catch {
      setState({
        status: 'ambiguous',
        error: { stage: 'server', cause: 'Error' },
        warnings: [],
      });
    } finally {
      running.current = false;
    }
  }

  async function checkStatus(): Promise<void> {
    if (poll === undefined || pendingId === null || polling.current !== null) return;
    const controller = new AbortController();
    polling.current = controller;
    setChecking(true);
    setStatusError(false);
    try {
      const next = await poll(pendingId, controller.signal);
      if (!controller.signal.aborted) setOperation(next);
    } catch {
      if (!controller.signal.aborted) setStatusError(true);
    } finally {
      polling.current = null;
      if (!controller.signal.aborted) setChecking(false);
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
            void checkStatus();
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

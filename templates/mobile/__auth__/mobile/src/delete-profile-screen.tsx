import { type ReactElement, useEffect, useRef, useState } from 'react';
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
type StatusError = 'unavailable' | 'expired' | null;
type Styles = ReturnType<typeof createStyles>;

const RESULT_MESSAGES = {
  erased: 'erasure.erased',
  pending: 'erasure.pending',
  ambiguous: 'erasure.ambiguous',
  'server-failure': 'erasure.serverFailure',
  'local-failure': 'erasure.localFailure',
  'signout-failure': 'erasure.signoutFailure',
} as const satisfies Record<ProductProfileErasureResult['status'], string>;

/** The translation key for the latest outcome; later stages win over earlier ones. */
function messageKey(
  state: State,
  statusError: StatusError,
  operation: ProfileErasureOperation | undefined,
): string | undefined {
  if (operation?.status === 'failed') return 'erasure.failed';
  if (operation?.status === 'completed') return 'erasure.erased';
  if (statusError === 'expired') return 'erasure.statusExpired';
  if (statusError === 'unavailable') return 'erasure.statusError';
  if (state === 'preparation-failure') return 'erasure.preparationFailure';
  if (state === 'erasing') return 'erasure.busy';
  return typeof state === 'string' ? undefined : RESULT_MESSAGES[state.status];
}

function isRetryable(state: State): boolean {
  if (state === 'preparation-failure') return true;
  return typeof state !== 'string' && (state.status === 'server-failure' || state.status === 'ambiguous');
}

interface StatusPolling {
  readonly operation: ProfileErasureOperation | undefined;
  readonly statusError: StatusError;
  readonly checking: boolean;
  readonly recheck: () => void;
}

function useStatusPolling(
  poll: DeleteProfileScreenProps['poll'],
  pendingId: string | null,
): StatusPolling {
  const [operation, setOperation] = useState<ProfileErasureOperation>();
  const [statusError, setStatusError] = useState<StatusError>(null);
  const [pollRound, setPollRound] = useState(0);
  const [finishedRound, setFinishedRound] = useState<number | null>(null);

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

  return {
    operation,
    statusError,
    checking: poll !== undefined && pendingId !== null && finishedRound !== pollRound,
    recheck: () => {
      setStatusError(null);
      setPollRound((round) => round + 1);
    },
  };
}

function useErasure(
  erase: DeleteProfileScreenProps['erase'],
): [State, (state: State) => void, () => Promise<void>] {
  const [state, setState] = useState<State>('idle');
  const running = useRef(false);
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
  return [state, setState, submit];
}

interface ErasureActionsProps {
  readonly state: State;
  readonly available: boolean;
  readonly canCheck: boolean;
  readonly polling: StatusPolling;
  readonly setState: (state: State) => void;
  readonly submit: () => Promise<void>;
  readonly styles: Styles;
}

function ErasureActions({
  state,
  available,
  canCheck,
  polling,
  setState,
  submit,
  styles,
}: ErasureActionsProps): ReactElement {
  const { t } = useTranslation('home');
  return (
    <>
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
      {isRetryable(state) ? (
        <ActionButton
          label={t('erasure.retry')}
          onPress={() => {
            void submit();
          }}
        />
      ) : null}
      {canCheck ? (
        <ActionButton
          disabled={polling.checking}
          label={t('erasure.checkStatus')}
          onPress={polling.recheck}
          secondary
        />
      ) : null}
    </>
  );
}

export function DeleteProfileScreen({
  erase,
  poll,
  available = true,
}: DeleteProfileScreenProps): ReactElement {
  const { t } = useTranslation('home');
  const { theme } = useTheme();
  const styles = createStyles(theme);
  const [state, setState, submit] = useErasure(erase);
  const heading = useRef<Text>(null);
  const status = useRef<Text>(null);
  const pendingId =
    typeof state !== 'string' && state.status === 'pending' ? state.receipt.operationId : null;
  const polling = useStatusPolling(poll, pendingId);
  const key = messageKey(state, polling.statusError, polling.operation);
  const message = key === undefined ? undefined : t(key);
  const canCheck =
    polling.statusError !== 'expired' &&
    poll !== undefined &&
    pendingId !== null &&
    (polling.operation === undefined || polling.operation.status === 'pending');

  useEffect(() => {
    focusAccessibilityElement(message === undefined ? heading : status);
    if (message !== undefined) announce(message);
    else if (state === 'confirming') announce(t('erasure.confirmation'));
  }, [message, state, t]);

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
      <ErasureActions
        state={state}
        available={available}
        canCheck={canCheck}
        polling={polling}
        setState={setState}
        submit={submit}
        styles={styles}
      />
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
